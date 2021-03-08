# Barbecue thermometer to Homie bridge

`cloudbbq-homie` is a service which runs on a Linux device to connect to a CloudBBQ or iBBQ
Bluetooth BBQ thermometer, and send its readings to an MQTT broker following the
[Homie convention](https://homieiot.github.io/). This allows you to control it via
[OpenHAB](https://www.openhab.org/) and other home automation software, log readings to InfluxDB
with [homie-influx](https://github.com/alsuren/mijia-homie/tree/master/homie-influx), or integrate
with various other [compatible tools](https://homieiot.github.io/implementations/#controller).

This is not an officially supported Google product.

## Usage

1. Copy `cloudbbq-homie.example.toml` to `cloudbbq-homie.toml` and edit it to configure your MQTT
   broker and other details. The comments there should explain what the fields do.
2. Turn on your BBQ thermometer.
3. Run `cloudbbq-homie` from the same directory as the config file.
4. Try connecting to your MQTT try connecting to your MQTT broker with a
   [Homie controller](https://homieiot.github.io/implementations/#controller) such as
   [HoDD](https://rroemhild.github.io/hodd/) to see your probe values.

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
