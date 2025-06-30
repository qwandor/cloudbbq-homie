// Copyright 2021 the cloudbbq-homie authors.
// This project is dual-licensed under Apache 2.0 and MIT terms.
// See LICENSE-APACHE and LICENSE-MIT for details.

mod bbq;
mod config;

use crate::bbq::Bbq;
use crate::config::{Config, get_tls_client_config};
use bluez_async::BluetoothSession;
use cloudbbq::find_devices;
use eyre::{Report, bail};
use futures::TryFutureExt;
use futures::future::try_join_all;
use rustls::ClientConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::{task, time, try_join};

const SCAN_DURATION: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> Result<(), Report> {
    stable_eyre::install()?;
    pretty_env_logger::init();
    color_backtrace::install();

    let config = Config::from_file()?;
    let tls_client_config = get_tls_client_config(&config.mqtt);

    // Connect a Bluetooth session.
    let (dbus_handle, session) = BluetoothSession::new().await?;

    let bbq_handle = run_system(&config, tls_client_config, &session);

    // Poll everything to completion, until the first one bombs out.
    let res: Result<_, Report> = try_join! {
        // If this ever finishes, we lost connection to D-Bus.
        dbus_handle.err_into(),
        bbq_handle.err_into(),
    };
    res?;

    Ok(())
}

async fn run_system(
    config: &Config,
    tls_client_config: Option<Arc<ClientConfig>>,
    session: &BluetoothSession,
) -> Result<(), Report> {
    log::info!("Starting discovery");
    session.start_discovery().await?;
    time::sleep(SCAN_DURATION).await;
    let devices = find_devices(session).await?;
    if devices.is_empty() {
        bail!("No devices found");
    }

    let mut join_handles = vec![];
    for device in devices {
        let bbq = Bbq::connect(session, device, config.to_owned()).await?;
        let handle = task::spawn(bbq.run(tls_client_config.clone()));
        join_handles.push(handle);
    }
    try_join_all(join_handles).await?;

    Ok(())
}
