use crate::i18n::{L10n, Lang};
use crate::types::*;
use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, ACCEPT, AUTHORIZATION};
use reqwest::Proxy;

pub struct Client {
    http: reqwest::Client,
    github_token: String,
    use_mirror: bool,
    lang: Lang,
}

impl Client {
    async fn cancellable_sleep(
        delay: std::time::Duration,
        cancel: Option<&CancelSignal>,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        while start.elapsed() < delay {
            if let Some(signal) = cancel {
                signal.checkpoint()?;
            }
            let remaining = delay.saturating_sub(start.elapsed());
            tokio::time::sleep(std::cmp::min(
                remaining,
                std::time::Duration::from_millis(100),
            ))
            .await;
        }
        Ok(())
    }

    pub fn new(config: &Config) -> Result<Self> {
        let t = L10n::new(Lang::from_str(&config.language));
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("snout/0.1");

        // 代理
        if config.proxy_enabled {
            let proxy = match config.proxy_type.as_str() {
                "http" | "https" => Proxy::all(format!("http://{}", config.proxy_address))?,
                "socks5" => Proxy::all(format!("socks5://{}", config.proxy_address))?,
                _ => {
                    eprintln!("⚠️ {}: {}", t.t("api.proxy_unknown"), config.proxy_type);
                    return Err(anyhow::anyhow!("{}", t.t("api.proxy_unknown")));
                }
            };
            builder = builder.proxy(proxy);
        }

        Ok(Self {
            http: builder.build()?,
            github_token: config.github_token.clone(),
            use_mirror: config.use_mirror,
            lang: Lang::from_str(&config.language),
        })
    }

    /// 无超时的 client (用于大文件下载)
    pub fn new_download_client(config: &Config) -> Result<Self> {
        let t = L10n::new(Lang::from_str(&config.language));
        let mut builder = reqwest::Client::builder().user_agent("snout/0.1");

        if config.proxy_enabled {
            let proxy = match config.proxy_type.as_str() {
                "http" | "https" => Proxy::all(format!("http://{}", config.proxy_address))?,
                "socks5" => Proxy::all(format!("socks5://{}", config.proxy_address))?,
                _ => return Err(anyhow::anyhow!("{}", t.t("api.proxy_unknown"))),
            };
            builder = builder.proxy(proxy);
        }

        Ok(Self {
            http: builder.build()?,
            github_token: config.github_token.clone(),
            use_mirror: config.use_mirror,
            lang: Lang::from_str(&config.language),
        })
    }

    fn github_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if !self.github_token.is_empty() {
            if let Ok(val) = format!("Bearer {}", self.github_token).parse() {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    fn cnb_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(val) = "application/vnd.cnb.web+json".parse() {
            headers.insert(ACCEPT, val);
        }
        if !self.github_token.is_empty() {
            if let Ok(val) = format!("Bearer {}", self.github_token).parse() {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    // ── GitHub Releases ──

    /// 获取 GitHub 分支头信息并构造归档下载信息
    pub async fn fetch_github_branch_archive(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        archive_name: &str,
        cancel: Option<&CancelSignal>,
    ) -> Result<UpdateInfo> {
        let t = L10n::new(self.lang);
        if let Some(signal) = cancel {
            signal.checkpoint()?;
        }
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/branches/{branch}");

        let resp = self
            .http
            .get(&url)
            .headers(self.github_headers())
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("{} {}", t.t("api.github_branch_status"), resp.status());
        }

        let branch_info: serde_json::Value = resp.json().await?;
        let sha = branch_info
            .get("commit")
            .and_then(|v| v.get("sha"))
            .and_then(|v| v.as_str())
            .with_context(|| t.t("api.github_branch_missing_sha").to_string())?;

        Ok(UpdateInfo {
            name: archive_name.into(),
            url: format!("https://github.com/{owner}/{repo}/archive/refs/heads/{branch}.zip"),
            update_time: String::new(),
            tag: sha.into(),
            description: format!("{owner}/{repo}@{branch}"),
            sha256: String::new(),
            size: 0,
        })
    }

    /// 获取 GitHub Releases (可选 tag 过滤)
    pub async fn fetch_github_releases(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
        cancel: Option<&CancelSignal>,
    ) -> Result<Vec<GitHubRelease>> {
        let t = L10n::new(self.lang);
        let url = if tag.is_empty() {
            format!("{GITHUB_API}/repos/{owner}/{repo}/releases?per_page=30")
        } else {
            format!("{GITHUB_API}/repos/{owner}/{repo}/releases/tags/{tag}")
        };

        let mut last_err = None;
        for attempt in 0..3 {
            if let Some(signal) = cancel {
                signal.checkpoint()?;
            }
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                Self::cancellable_sleep(delay, cancel).await?;
            }

            let resp = self
                .http
                .get(&url)
                .headers(self.github_headers())
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    if tag.is_empty() {
                        let releases: Vec<GitHubRelease> = r.json().await?;
                        return Ok(releases);
                    } else {
                        let release: GitHubRelease = r.json().await?;
                        return Ok(vec![release]);
                    }
                }
                Ok(r) => {
                    last_err = Some(anyhow::anyhow!(
                        "{} {}",
                        t.t("api.github_status"),
                        r.status()
                    ));
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("{}: {e}", t.t("api.github_request_failed")));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{}", t.t("api.github_request_failed"))))
    }

    // ── CNB 镜像 ──

    /// 获取 CNB Release
    pub async fn fetch_cnb_release(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
        cancel: Option<&CancelSignal>,
    ) -> Result<GitHubRelease> {
        let t = L10n::new(self.lang);
        let url = format!("{CNB_BASE}/{owner}/{repo}/-/releases/tags/{tag}");

        let mut last_err = None;
        for attempt in 0..3 {
            if let Some(signal) = cancel {
                signal.checkpoint()?;
            }
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                Self::cancellable_sleep(delay, cancel).await?;
            }

            let resp = self.http.get(&url).headers(self.cnb_headers()).send().await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    let release: GitHubRelease = r.json().await?;
                    return Ok(release);
                }
                Ok(r) => {
                    last_err = Some(anyhow::anyhow!("{} {}", t.t("api.cnb_status"), r.status()));
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("{}: {e}", t.t("api.cnb_request_failed")));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{}", t.t("api.cnb_request_failed"))))
    }

    /// 获取 CNB 最新 tag
    #[allow(dead_code)]
    pub async fn fetch_cnb_latest_tag(
        &self,
        owner: &str,
        repo: &str,
        cancel: Option<&CancelSignal>,
    ) -> Result<String> {
        let t = L10n::new(self.lang);
        if let Some(signal) = cancel {
            signal.checkpoint()?;
        }
        let url = format!("{CNB_BASE}/{owner}/{repo}/-/releases?page=1&per_page=1");
        let resp = self
            .http
            .get(&url)
            .headers(self.cnb_headers())
            .send()
            .await?;

        let releases: Vec<GitHubRelease> = resp.json().await?;
        releases
            .into_iter()
            .next()
            .map(|r| r.tag_name)
            .with_context(|| t.t("api.cnb_no_release").to_string())
    }

    // ── 通用下载 ──

    /// 流式下载到文件，支持进度回调和重试
    pub async fn download_file(
        &self,
        url: &str,
        dest: &std::path::Path,
        cancel: Option<&CancelSignal>,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<()> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let t = L10n::new(self.lang);
        let mut last_err = None;
        for attempt in 0..3 {
            if let Some(signal) = cancel {
                signal.checkpoint()?;
            }
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                eprintln!(
                    "⚠️ {}: {}s ({}/3)...",
                    t.t("api.download_retry"),
                    delay.as_secs(),
                    attempt + 1
                );
                Self::cancellable_sleep(delay, cancel).await?;
            }

            // 每次重试重新创建文件 (截断)
            let resp = match self.http.get(url).send().await {
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

            let total = resp.content_length();
            let mut file = tokio::fs::File::create(dest).await?;
            let mut stream = resp.bytes_stream();
            let mut downloaded: u64 = 0;
            let mut stream_err = None;

            while let Some(chunk) = stream.next().await {
                if let Some(signal) = cancel {
                    signal.checkpoint()?;
                }
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
            return Ok(());
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{}", t.t("err.download_failed"))))
    }

    pub fn use_mirror(&self) -> bool {
        self.use_mirror
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

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

    #[test]
    fn client_new_accepts_supported_proxy_types() {
        for proxy_type in ["http", "https", "socks5"] {
            let mut config = base_config();
            config.proxy_enabled = true;
            config.proxy_type = proxy_type.into();
            assert!(Client::new(&config).is_ok(), "proxy_type={proxy_type}");
        }
    }

    #[test]
    fn client_new_rejects_unknown_proxy_type() {
        let mut config = base_config();
        config.proxy_enabled = true;
        config.proxy_type = "nope".into();
        assert!(Client::new(&config).is_err());
    }

    #[test]
    fn download_client_accepts_supported_proxy_types() {
        for proxy_type in ["http", "https", "socks5"] {
            let mut config = base_config();
            config.proxy_enabled = true;
            config.proxy_type = proxy_type.into();
            assert!(
                Client::new_download_client(&config).is_ok(),
                "proxy_type={proxy_type}"
            );
        }
    }

    #[test]
    fn download_client_rejects_unknown_proxy_type() {
        let mut config = base_config();
        config.proxy_enabled = true;
        config.proxy_type = "bad".into();
        assert!(Client::new_download_client(&config).is_err());
    }

    #[test]
    fn github_headers_omit_auth_when_token_missing() {
        let client = Client::new(&base_config()).expect("client");
        let headers = client.github_headers();
        assert!(!headers.contains_key(AUTHORIZATION));
    }

    #[test]
    fn github_headers_include_bearer_token() {
        let mut config = base_config();
        config.github_token = "secret".into();
        let client = Client::new(&config).expect("client");
        let headers = client.github_headers();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap(),
            &"Bearer secret".parse::<HeaderValue>().unwrap()
        );
    }

    #[test]
    fn cnb_headers_always_include_accept_header() {
        let client = Client::new(&base_config()).expect("client");
        let headers = client.cnb_headers();
        assert_eq!(
            headers.get(ACCEPT).unwrap(),
            &"application/vnd.cnb.web+json"
                .parse::<HeaderValue>()
                .unwrap()
        );
    }

    #[test]
    fn cnb_headers_include_optional_bearer_token() {
        let mut config = base_config();
        config.github_token = "token".into();
        let client = Client::new(&config).expect("client");
        let headers = client.cnb_headers();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap(),
            &"Bearer token".parse::<HeaderValue>().unwrap()
        );
    }
}
