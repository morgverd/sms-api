# SMS API

Send and receive SMS messages via a GSM modem.

## Features
- Optional HTTP server to send modem operations and access database.
- Handles SMS delivery reports, updating send status when received.
- HTTP webhook support for different events (incoming, outgoing, delivery).
- **All incoming and outgoing SMS message content is stored encrypted.**

## Hardware

You'll need some form of GSM modem that allows for serial connection.
I use (and this project has only been tested with) a [Waveshare GSM Pi Hat](https://www.waveshare.com/gsm-gprs-gnss-hat.htm) on a Raspberry Pi.

## Configuration

Here is a simple configuration file that enables the HTTP API and specifies the modem device.
The only truly required options here are the `database` fields.

A full example with all annotated fields can be found [here](config.example.yaml).

> To use the config file, simply specify `./sms-api -c config.yaml`. See `./sms-api -h` for more information.

```yaml
# Specify the SQLite database path and encryption key used when storing/accessing message content.
database:
  database_url: /home/pi/sms-database.db
  encryption_key: "aGVsbG9fdGhlcmVfaG93X2FyZV95b3VfdG9kYXk/Pz8="

# This is the default device, but it can be easily changed.
modem:
  device: /dev/ttyS0

# By default, the HTTP server is disabled.
http:
  enabled: true

# Adds a webhook which will receive all events.
webhooks:
  - url: https://webhook.my-site.org
    headers:
      Authorization: hello
    events:
      - incoming
      - outgoing
      - delivery
```

## Todo

- Support both Postgres and SQLite as database options (or just Postgres).
- Make database message storage entirely optional?
- Add API basic authentication.
- Properly document API routes.