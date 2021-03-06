use bluez_async::MacAddress;
use eyre::Report;
use rumqttc::{MqttOptions, TlsConfiguration, Transport};
use rustls::ClientConfig;
use serde::de::Error as _;
use serde::{Deserialize as _, Deserializer};
use serde_derive::Deserialize;
use stable_eyre::eyre::WrapErr;
use std::collections::HashMap;
use std::fs::read_to_string;
use std::sync::Arc;

const DEFAULT_MQTT_PREFIX: &str = "homie";
const DEFAULT_MQTT_CLIENT_PREFIX: &str = "cloudbbq";
const DEFAULT_DEVICE_ID_PREFIX: &str = "cloudbbq";
const DEFAULT_HOST: &str = "test.mosquitto.org";
const DEFAULT_PORT: u16 = 1883;
const CONFIG_FILENAME: &str = "cloudbbq-homie.toml";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub homie: HomieConfig,
    #[serde(deserialize_with = "de_device_map", rename = "device")]
    pub devices: HashMap<MacAddress, DeviceConfig>,
}

impl Config {
    pub fn from_file() -> Result<Config, Report> {
        Config::read(CONFIG_FILENAME)
    }

    fn read(filename: &str) -> Result<Config, Report> {
        let config_file =
            read_to_string(filename).wrap_err_with(|| format!("Reading {}", filename))?;
        Ok(toml::from_str(&config_file)?)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_prefix: String,
}

impl Default for MqttConfig {
    fn default() -> MqttConfig {
        MqttConfig {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            use_tls: false,
            username: None,
            password: None,
            client_prefix: DEFAULT_MQTT_CLIENT_PREFIX.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HomieConfig {
    pub device_id_prefix: String,
    pub prefix: String,
}

impl Default for HomieConfig {
    fn default() -> HomieConfig {
        HomieConfig {
            device_id_prefix: DEFAULT_DEVICE_ID_PREFIX.to_owned(),
            prefix: DEFAULT_MQTT_PREFIX.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceConfig {
    pub name: Option<String>,
    pub probe_names: Vec<String>,
}

pub fn de_device_map<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<HashMap<MacAddress, DeviceConfig>, D::Error> {
    let map: HashMap<String, DeviceConfig> = HashMap::deserialize(d)?;
    map.into_iter()
        .map(|(mac_address, device_config)| {
            Ok((
                mac_address.parse().map_err(D::Error::custom)?,
                device_config,
            ))
        })
        .collect()
}

/// Construct a `ClientConfig` for TLS connections to the MQTT broker, if TLS is enabled.
pub fn get_tls_client_config(config: &MqttConfig) -> Option<Arc<ClientConfig>> {
    if config.use_tls {
        let mut client_config = ClientConfig::new();
        client_config.root_store = rustls_native_certs::load_native_certs()
            .expect("Failed to load platform certificates.");
        Some(Arc::new(client_config))
    } else {
        None
    }
}

/// Construct the `MqttOptions` for connecting to the MQTT broker based on configuration options or
/// defaults.
pub fn get_mqtt_options(
    config: &MqttConfig,
    client_name_suffix: &str,
    tls_client_config: Option<Arc<ClientConfig>>,
) -> MqttOptions {
    let client_name = format!("{}-{}", config.client_prefix, client_name_suffix);
    let mut mqtt_options = MqttOptions::new(client_name, &config.host, config.port);
    mqtt_options.set_keep_alive(5);

    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        mqtt_options.set_credentials(username, password);
    }

    if let Some(client_config) = tls_client_config {
        mqtt_options.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(
            client_config,
        )));
    }

    mqtt_options
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parsing the example config file should not give any errors.
    #[test]
    fn example_config() {
        Config::read("cloudbbq-homie.example.toml").unwrap();
    }

    /// Parsing an empty config file should not give any errors.
    #[test]
    fn empty_config() {
        toml::from_str::<Config>("").unwrap();
    }
}
