# Barbecue thermometer to Homie bridge

`cloudbbq-homie` is a service which runs on a Linux device to connect to a CloudBBQ or iBBQ
Bluetooth BBQ thermometer, and send its readings to an MQTT broker following the
[Homie convention](https://homieiot.github.io/). This allows you to control it via
[OpenHAB](https://www.openhab.org/) and other home automation software, log readings to InfluxDB
with [homie-influx](https://github.com/alsuren/mijia-homie/tree/master/homie-influx), or integrate
with various other [compatible tools](https://homieiot.github.io/implementations/#controller).

This is not an officially supported Google product.

## Installation

If you want to run `cloudbbq-homie` as a system service, you can install the latest release from our
Debian repository:

```sh
$ curl -L https://homiers.jfrog.io/artifactory/api/security/keypair/public/repositories/homie-rs | sudo apt-key add -
$ echo "deb https://homiers.jfrog.io/artifactory/homie-rs stable main" | sudo tee /etc/apt/sources.list.d/homie-rs.list
$ sudo apt update && sudo apt install cloudbbq-homie
```

Alternatively, you may install with cargo install:

```sh
$ cargo install cloudbbq-homie
```

## Usage

1. Copy `cloudbbq-homie.example.toml` to `cloudbbq-homie.toml` and edit it to configure your MQTT
   broker and other details. The comments there should explain what the fields do. (If you installed
   the Debian package, the config file is installed as `/etc/cloudbbq-homie/cloudbbq-homie.toml`.)
2. Turn on your BBQ thermometer.
3. Run `cloudbbq-homie` from the same directory as the config file.
4. Try connecting to your MQTT broker with a
   [Homie controller](https://homieiot.github.io/implementations/#controller) such as
   [HoDD](https://rroemhild.github.io/hodd/) to see your probe values. Or use
   [homie-influx](https://crates.io/crates/homie-influx) to store the readings in InfluxDB so you
   can draw charts with Grafana.

## License

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the
work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.

See the [contributing guidelines](CONTRIBUTING.md) for more details.
