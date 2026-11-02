//! Read-only host locale port (FR-016, D118). No account or preference is exposed remotely.
use async_trait::async_trait;
use dgos_types::Locale;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct LocaleBootstrap {
    pub locale: Locale,
    pub catalog_version: &'static str,
}

#[async_trait]
pub trait LocaleSource: Send + Sync + 'static {
    async fn bootstrap(&self) -> Result<LocaleBootstrap, ()>;
}

pub struct DefaultLocaleSource;

#[async_trait]
impl LocaleSource for DefaultLocaleSource {
    async fn bootstrap(&self) -> Result<LocaleBootstrap, ()> {
        Ok(LocaleBootstrap {
            locale: Locale::ZhCn,
            catalog_version: "v1",
        })
    }
}
