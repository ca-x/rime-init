use super::Client;
use crate::i18n::L10n;
use crate::types::*;
use anyhow::Result;
use std::path::{Path, PathBuf};

struct ParallelDownloadContext<'a> {
    url: &'a str,
    tmp_path: &'a Path,
    total_size: u64,
    threads: usize,
    cancel: Option<&'a CancelSignal>,
    prefer_direct_mirror: bool,
}

impl Client {
    pub async fn download_file(
        &self,
        url: &str,
        dest: &std::path::Path,
        config: &Config,
        cancel: Option<&CancelSignal>,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<()> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let t = L10n::new(self.lang);
        let mut last_err = None;
        let tmp_path = temp_download_path(dest);
        let prefer_direct_mirror = self.has_proxy() && self.is_mirror_url(url);
        let desired_threads = config.download_threads.clamp(1, 8);
        for attempt in 0..3 {
            if let Some(signal) = cancel {
                signal.checkpoint()?;
            }
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                crate::feedback::warn(format!(
                    "⚠️ {}: {}s ({}/3)...",
                    t.t("api.download_retry"),
                    delay.as_secs(),
                    attempt + 1
                ));
                Client::cancellable_sleep(delay, cancel).await?;
            }

            let resume_from = tokio::fs::metadata(&tmp_path)
                .await
                .map(|meta| meta.len())
                .unwrap_or(0);

            let resp = match self
                .send_download_request(url, resume_from, cancel, prefer_direct_mirror)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!(
                        "{}: {e}",
                        t.t("api.download_request_failed")
                    ));
                    continue;
                }
            };

            if !resp.status().is_success() {
                last_err = Some(anyhow::anyhow!(
                    "{} {}",
                    t.t("api.download_http_failed"),
                    resp.status()
                ));
                continue;
            }

            let total = response_total_length(&resp, resume_from);
            if resume_from == 0 && desired_threads > 1 {
                if let Some(total_size) = total {
                    if self.supports_parallel_download(&resp)
                        && self
                            .download_file_parallel(
                                ParallelDownloadContext {
                                    url,
                                    tmp_path: &tmp_path,
                                    total_size,
                                    threads: desired_threads,
                                    cancel,
                                    prefer_direct_mirror,
                                },
                                &mut progress,
                            )
                            .await
                            .is_ok()
                    {
                        tokio::fs::rename(&tmp_path, dest).await?;
                        return Ok(());
                    }
                }
            }

            let mut downloaded = resume_from;
            let mut file =
                if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0 {
                    tokio::fs::OpenOptions::new()
                        .append(true)
                        .open(&tmp_path)
                        .await?
                } else {
                    downloaded = 0;
                    tokio::fs::File::create(&tmp_path).await?
                };
            let mut stream = resp.bytes_stream();
            let mut stream_err = None;

            loop {
                let next_chunk = if let Some(signal) = cancel {
                    tokio::select! {
                        chunk = stream.next() => chunk,
                        result = Client::wait_for_cancel(signal) => {
                            result?;
                            unreachable!()
                        }
                    }
                } else {
                    stream.next().await
                };

                let Some(chunk) = next_chunk else {
                    break;
                };
                match chunk {
                    Ok(c) => {
                        if let Err(e) = file.write_all(&c).await {
                            stream_err = Some(e);
                            break;
                        }
                        downloaded += c.len() as u64;
                        progress(downloaded, total);
                    }
                    Err(e) => {
                        stream_err = Some(std::io::Error::other(e));
                        break;
                    }
                }
            }

            if let Some(e) = stream_err {
                last_err = Some(anyhow::anyhow!("{}: {e}", t.t("api.download_interrupted")));
                continue;
            }

            file.flush().await?;
            tokio::fs::rename(&tmp_path, dest).await?;
            return Ok(());
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{}", t.t("err.download_failed"))))
    }
}

impl Client {
    fn supports_parallel_download(&self, resp: &reqwest::Response) -> bool {
        if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            return true;
        }
        resp.headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("bytes"))
    }

    async fn send_download_request(
        &self,
        url: &str,
        resume_from: u64,
        cancel: Option<&CancelSignal>,
        prefer_direct_mirror: bool,
    ) -> Result<reqwest::Response> {
        let mut direct_error = None;

        if prefer_direct_mirror {
            let mut direct = self.http_direct.get(url);
            if resume_from > 0 {
                direct = direct.header("Range", format!("bytes={resume_from}-"));
            }
            match Client::await_with_cancel(direct.send(), cancel).await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    direct_error = Some(anyhow::anyhow!(
                        "direct mirror download returned {}",
                        response.status()
                    ));
                }
                Err(error) => direct_error = Some(error),
            }
        }

        let mut proxied = self.http.get(url);
        if resume_from > 0 {
            proxied = proxied.header("Range", format!("bytes={resume_from}-"));
        }
        match Client::await_with_cancel(proxied.send(), cancel).await {
            Ok(response) => Ok(response),
            Err(error) if direct_error.is_some() => Err(anyhow::anyhow!(
                "mirror download failed without proxy ({}) and with proxy ({error})",
                direct_error.expect("direct error")
            )),
            Err(error) => Err(error),
        }
    }

    async fn download_file_parallel(
        &self,
        context: ParallelDownloadContext<'_>,
        progress: &mut impl FnMut(u64, Option<u64>),
    ) -> Result<()> {
        if context.threads <= 1 || context.total_size < 512 * 1024 {
            anyhow::bail!("parallel download not beneficial");
        }

        let threads = context
            .threads
            .min(context.total_size.div_ceil(512_000) as usize)
            .max(1);
        if threads <= 1 {
            anyhow::bail!("parallel download collapsed to single stream");
        }

        let chunk_size = context.total_size.div_ceil(threads as u64);
        let progress_state = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut handles = Vec::with_capacity(threads);

        for index in 0..threads {
            let start = index as u64 * chunk_size;
            if start >= context.total_size {
                break;
            }
            let end = (start + chunk_size)
                .min(context.total_size)
                .saturating_sub(1);
            let part_path = parallel_part_path(context.tmp_path, index);
            let progress_state = progress_state.clone();
            let url = context.url.to_string();
            let cancel = context.cancel.cloned();
            let this = self.clone_download_client()?;
            let prefer_direct_mirror = context.prefer_direct_mirror;

            handles.push(tokio::spawn(async move {
                let downloaded = this
                    .download_range_part(
                        &url,
                        &part_path,
                        start,
                        end,
                        cancel.as_ref(),
                        prefer_direct_mirror,
                    )
                    .await?;
                progress_state.fetch_add(downloaded, std::sync::atomic::Ordering::SeqCst);
                Ok::<PathBuf, anyhow::Error>(part_path)
            }));
        }

        let mut part_paths = Vec::new();
        for handle in handles {
            let part_path = handle.await??;
            let downloaded = progress_state.load(std::sync::atomic::Ordering::SeqCst);
            progress(downloaded.min(context.total_size), Some(context.total_size));
            part_paths.push(part_path);
        }
        part_paths.sort();

        merge_parallel_parts(context.tmp_path, &part_paths).await?;
        for part_path in part_paths {
            let _ = tokio::fs::remove_file(part_path).await;
        }
        Ok(())
    }

    async fn download_range_part(
        &self,
        url: &str,
        part_path: &Path,
        start: u64,
        end: u64,
        cancel: Option<&CancelSignal>,
        prefer_direct_mirror: bool,
    ) -> Result<u64> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let resp = self
            .send_download_request_with_range(url, start, end, cancel, prefer_direct_mirror)
            .await?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            anyhow::bail!(
                "range download requires 206 response, got {}",
                resp.status()
            );
        }

        let mut file = tokio::fs::File::create(part_path).await?;
        let mut stream = resp.bytes_stream();
        let mut downloaded = 0u64;
        while let Some(chunk) = if let Some(signal) = cancel {
            tokio::select! {
                chunk = stream.next() => chunk,
                result = Client::wait_for_cancel(signal) => {
                    result?;
                    unreachable!()
                }
            }
        } else {
            stream.next().await
        } {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
        }
        file.flush().await?;
        Ok(downloaded)
    }

    async fn send_download_request_with_range(
        &self,
        url: &str,
        start: u64,
        end: u64,
        cancel: Option<&CancelSignal>,
        prefer_direct_mirror: bool,
    ) -> Result<reqwest::Response> {
        let range_header = format!("bytes={start}-{end}");
        let mut direct_error = None;

        if prefer_direct_mirror {
            let direct = self
                .http_direct
                .get(url)
                .header(reqwest::header::RANGE, range_header.clone())
                .send();
            match Client::await_with_cancel(direct, cancel).await {
                Ok(response)
                    if response.status().is_success()
                        || response.status() == reqwest::StatusCode::PARTIAL_CONTENT =>
                {
                    return Ok(response);
                }
                Ok(response) => {
                    direct_error = Some(anyhow::anyhow!(
                        "direct mirror ranged download returned {}",
                        response.status()
                    ));
                }
                Err(error) => direct_error = Some(error),
            }
        }

        let proxied = self
            .http
            .get(url)
            .header(reqwest::header::RANGE, range_header)
            .send();
        match Client::await_with_cancel(proxied, cancel).await {
            Ok(response) => Ok(response),
            Err(error) if direct_error.is_some() => Err(anyhow::anyhow!(
                "mirror ranged download failed without proxy ({}) and with proxy ({error})",
                direct_error.expect("direct error")
            )),
            Err(error) => Err(error),
        }
    }

    fn clone_download_client(&self) -> Result<Self> {
        Ok(Self {
            http: self.http.clone(),
            http_direct: self.http_direct.clone(),
            github_token: self.github_token.clone(),
            use_mirror: self.use_mirror,
            lang: self.lang,
            has_proxy: self.has_proxy,
        })
    }
}

fn temp_download_path(dest: &Path) -> std::path::PathBuf {
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    dest.with_file_name(format!("{file_name}.tmp"))
}

fn parallel_part_path(dest: &Path, index: usize) -> std::path::PathBuf {
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    dest.with_file_name(format!("{file_name}.part{index}"))
}

async fn merge_parallel_parts(dest: &Path, part_paths: &[PathBuf]) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut output = tokio::fs::File::create(dest).await?;
    for part_path in part_paths {
        let mut input = tokio::fs::File::open(part_path).await?;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).await?;
        }
    }
    output.flush().await?;
    Ok(())
}

fn response_total_length(resp: &reqwest::Response, resume_from: u64) -> Option<u64> {
    if let Some(range) = resp.headers().get(reqwest::header::CONTENT_RANGE) {
        if let Ok(range) = range.to_str() {
            if let Some(total) = range.split('/').nth(1).and_then(|v| v.parse::<u64>().ok()) {
                return Some(total);
            }
        }
    }
    resp.content_length().map(|len| {
        if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            len + resume_from
        } else {
            len
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn base_config() -> Config {
        Config {
            github_token: String::new(),
            proxy_enabled: false,
            proxy_type: "socks5".into(),
            proxy_address: "127.0.0.1:1080".into(),
            language: "en".into(),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn download_file_resumes_from_partial_tmp_file() {
        let full_content = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match server.accept().await {
                    Ok(value) => value,
                    Err(_) => break,
                };
                let mut buf = [0u8; 1024];
                let n = match tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                let request = String::from_utf8_lossy(&buf[..n]);
                let range = request
                    .lines()
                    .find(|line| line.starts_with("Range:"))
                    .map(|line| line.trim().to_string());
                if let Some(range) = range {
                    assert!(range.contains("bytes=10-"));
                    let body = &full_content[10..];
                    let response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 10-35/36\r\n\r\n",
                        body.len()
                    );
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, body).await;
                } else {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                        full_content.len()
                    );
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, full_content).await;
                }
            }
        });

        let client = Client::new_download_client(&base_config()).expect("client");
        let dir = std::env::temp_dir().join("snout-api-resume-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("asset.bin");
        let tmp = temp_download_path(&dest);
        std::fs::write(&tmp, &full_content[..10]).unwrap();

        client
            .download_file(
                &format!("http://{addr}/asset"),
                &dest,
                &base_config(),
                None,
                |_downloaded, _total| {},
            )
            .await
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), full_content);
        assert!(!tmp.exists());

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_file_restarts_when_server_ignores_range() {
        let content = b"complete file content";
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match server.accept().await {
                    Ok(value) => value,
                    Err(_) => break,
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    content.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, content).await;
            }
        });

        let client = Client::new_download_client(&base_config()).expect("client");
        let dir = std::env::temp_dir().join("snout-api-no-resume-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("asset.bin");
        let tmp = temp_download_path(&dest);
        std::fs::write(&tmp, b"partial").unwrap();

        client
            .download_file(
                &format!("http://{addr}/asset"),
                &dest,
                &base_config(),
                None,
                |_downloaded, _total| {},
            )
            .await
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), content);
        assert!(!tmp.exists());

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_file_cancels_while_waiting_for_response() {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let _ = server.accept().await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });

        let client = Client::new_download_client(&base_config()).expect("client");
        let dir = std::env::temp_dir().join("snout-api-cancel-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("asset.bin");
        let cancel = CancelSignal::new();
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            client
                .download_file(
                    &format!("http://{addr}/asset"),
                    &dest,
                    &base_config(),
                    Some(&cancel_clone),
                    |_downloaded, _total| {},
                )
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancel.cancel();

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("download task should stop quickly")
            .expect("join");

        assert!(result.is_err());

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_file_uses_parallel_ranges_when_enabled() {
        let content = (0..2_000_000u32)
            .map(|n| (n % 251) as u8)
            .collect::<Vec<_>>();
        let server_content = content.clone();
        let observed_ranges = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed_ranges_server = observed_ranges.clone();

        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match server.accept().await {
                    Ok(value) => value,
                    Err(_) => break,
                };
                let mut buf = [0u8; 4096];
                let n = match tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                let request = String::from_utf8_lossy(&buf[..n]);
                let range_header = request
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.trim().eq_ignore_ascii_case("range") {
                            Some(value.trim())
                        } else {
                            None
                        }
                    })
                    .map(str::trim)
                    .map(str::to_string);

                if let Some(range) = range_header {
                    observed_ranges_server.lock().unwrap().push(range.clone());
                    let range = range.trim_start_matches("bytes=");
                    let (start, end) = range.split_once('-').unwrap();
                    let start: usize = start.parse().unwrap();
                    let end: usize = end.parse().unwrap();
                    let body = &server_content[start..=end];
                    let response = format!(
                        "HTTP/1.1 206 Partial Content\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
                        body.len(),
                        start,
                        end,
                        server_content.len()
                    );
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, body).await;
                } else {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\n\r\n",
                        server_content.len()
                    );
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, &server_content).await;
                }
            }
        });

        let mut config = base_config();
        config.download_threads = 4;
        let client = Client::new_download_client(&config).expect("client");
        let dir = std::env::temp_dir().join("snout-api-parallel-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("asset.bin");

        client
            .download_file(
                &format!("http://{addr}/asset"),
                &dest,
                &config,
                None,
                |_downloaded, _total| {},
            )
            .await
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), content);
        assert!(observed_ranges.lock().unwrap().len() >= 2);

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
