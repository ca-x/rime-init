mod download;
mod releases;

use crate::i18n::{L10n, Lang};
use crate::types::*;
use anyhow::Result;
use reqwest::header::{HeaderMap, ACCEPT, AUTHORIZATION};
use reqwest::Proxy;
pub struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) github_token: String,
    use_mirror: bool,
    pub(crate) lang: Lang,
}

impl Client {
    pub(crate) async fn wait_for_cancel(cancel: &CancelSignal) -> Result<()> {
        loop {
            cancel.checkpoint()?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    pub(crate) async fn await_with_cancel<T, F>(
        future: F,
        cancel: Option<&CancelSignal>,
    ) -> Result<T>
    where
        F: std::future::Future<Output = reqwest::Result<T>>,
    {
        if let Some(signal) = cancel {
            tokio::select! {
                result = future => result.map_err(Into::into),
                result = Self::wait_for_cancel(signal) => result.and_then(|_| unreachable!()),
            }
        } else {
            future.await.map_err(Into::into)
        }
    }

    pub(crate) async fn cancellable_sleep(
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

    pub(crate) fn github_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if !self.github_token.is_empty() {
            if let Ok(val) = format!("Bearer {}", self.github_token).parse() {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    pub(crate) fn cnb_headers(&self) -> HeaderMap {
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
