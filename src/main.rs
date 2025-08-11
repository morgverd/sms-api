mod modem;
mod http;
mod sms;
mod config;
pub mod webhooks;
pub mod app;

use std::path::PathBuf;
use anyhow::{Context, Result};
use clap::Parser;
use crate::app::AppHandles;

#[cfg(feature = "sentry")]
use {
    std::borrow::Cow,
    std::sync::Arc,
    std::time::Duration
};


#[cfg(feature = "sentry")]
pub type SentryGuard = Option<sentry::ClientInitGuard>;

#[cfg(not(feature = "sentry"))]
pub type SentryGuard = Option<()>;

#[derive(Parser)]
#[command(name = "sms-api")]
#[command(about = "A HTTP API that accepts and sends SMS messages.")]
#[command(version = if cfg!(feature = "sentry") {
    concat!(env!("CARGO_PKG_VERSION"), "+sentry")
} else {
    env!("CARGO_PKG_VERSION")
})]
struct CliArguments {

    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>
}

fn set_boxed_logger(boxed_logger: Box<dyn log::Log>) -> Result<()> {
    log::set_boxed_logger(boxed_logger).context("Failed to set logger")?;
    log::set_max_level(log::LevelFilter::Trace);
    Ok(())
}

#[cfg(feature = "sentry")]
fn init_sentry(
    config: &config::SentryConfig,
    logger: env_logger::Logger
) -> Result<sentry::ClientInitGuard> {
    log::debug!("Initializing Sentry integration");

    let sentry_logger = sentry_log::SentryLogger::with_dest(logger);
    set_boxed_logger(Box::new(sentry_logger))?;

    let panic_integration = sentry_panic::PanicIntegration::default().add_extractor(|_| None);
    let guard = sentry::init((config.dsn.clone(), sentry::ClientOptions {
        environment: config.environment.clone().map(Cow::Owned),
        server_name: config.server_name.clone().map(Cow::Owned),
        debug: config.debug,
        send_default_pii: config.send_default_pii,
        release: sentry::release_name!(),
        integrations: vec![Arc::new(panic_integration)],
        before_send: Some(Arc::new(|event| {
            log::warn!(
                "Sending to Sentry: {}",
                event.message.as_deref().or_else(|| {
                    event.exception.values.iter()
                        .filter_map(|e| e.value.as_deref())
                        .next()
                }).unwrap_or("Unknown!")
            );
            Some(event)
        })),
        ..Default::default()
    }));

    log::info!("Sentry integration initialized");
    Ok(guard)
}

fn main() -> Result<()> {
    let logger = env_logger::Builder::from_default_env().build();
    let args = CliArguments::parse();
    let config = config::AppConfig::load(args.config)?;

    #[cfg(feature = "sentry")]
    let _sentry_guard: SentryGuard = match config.sentry.as_ref() {
        Some(sentry_config) => Some(init_sentry(sentry_config, logger)?),
        None => set_boxed_logger(Box::new(logger)).map(|_| None)?
    };

    #[cfg(not(feature = "sentry"))]
    let _sentry_guard: SentryGuard = set_boxed_logger(Box::new(logger)).map(|_| None)?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let handles = AppHandles::create(config, _sentry_guard).await?;
            handles.run().await;

            #[cfg(feature = "sentry")]
            {
                log::info!("Flushing Sentry events before shutdown...");
                if let Some(client) = sentry::Hub::current().client() {
                    client.flush(Some(Duration::from_secs(2)));
                }
            }

            Ok(())
        })
}