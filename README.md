# SMS API

Send and receive SMS messages via a GSM modem.

## Features

- Optional HTTP server to send modem operations and access database (with optional token authorization).
- HTTP webhook support for different events (incoming, outgoing, delivery).
- **All incoming and outgoing SMS message content is stored encrypted.**
- Handles SMS delivery reports, updating send status when received.
- Allows unsolicited notifications to interrupt command execution for immediate updates.
- Optional Sentry client integration (must be built with `sentry` feature).
- Provides [pdu-rs](https://github.com/morgverd/pdu-rs) crate for SMS PDU parsing.

### HTTP Routes

| Route                       | AT Command | Description                                                                              |
|-----------------------------|------------|------------------------------------------------------------------------------------------|
| `POST /db/sms`              | -          | Query messages to and from a `phone_number` with pagination.                             |
| `POST /db/latest-numbers`   | -          | Query all latest numbers (sender or receiver) with optional pagination.                  |
| `POST /db/delivery-reports` | -          | Query all delivery reports for a `message_id` with optional pagination.                  |
| `POST /sms/send`            | `AT+CMGS`  | Send message `content` with a `to` target.                                               |
| `GET /sms/network-status`   | `AT+CREG?` | Get information about the registration status and access technology of the serving cell. |
| `GET /sms/signal-strength`  | `AT+CSQ`   | Get signal strength `rssi` and `ber` values.                                             |
| `GET /sms/network-operator` | `AT+COPS?` | Get the network operator ID, status and name.                                            |
| `GET /sms/service-provider` | `AT+CSPN?` | Get the the service provider name from the SIM.                                          |
| `GET /sms/battery-level`    | `AT+CBC`   | Get the device battery `status`, `charge` and `voltage`.                                 |

## Examples

### [ChatGPT SMS](./examples/chatgpt-sms)

Create a HTTP webhook server to accept `incoming` events and then send a ChatGPT generated reply with basic message history. This is probably a crime against humanity,
and it shouldn't actually be used, but it's a good example of using webhooks in a workflow.

> Which, to my knowledge, is the first ChatGPT SMS implementation running directly through a modem!

## Hardware

You'll need some form of GSM modem that allows for serial connection.
I use (and this project has only been tested with) a [Waveshare GSM Pi Hat](https://www.waveshare.com/gsm-gprs-gnss-hat.htm) on a Raspberry Pi.
Many SIMs require elaborate network registration, so you'd have to do that first.

## Configuration

Here is a simple configuration file that enables the HTTP API and specifies the modem device.
The only truly required options here are the `database` fields.

A full example with all annotated fields can be found [here](config.example.toml).

> To use the config file, simply specify `./sms-api -c config.toml`. See `./sms-api -h` for more information.

```toml
# Specify the SQLite database path and encryption key used when storing/accessing message content.
[database]
database_url = "/home/pi/sms-database.db"
encryption_key = "aGVsbG9fdGhlcmVfaG93X2FyZV95b3VfdG9kYXk/Pz8="

# This is the default device, but it can be easily changed.
[modem]
device = "/dev/ttyS0"

# By default, the HTTP server is disabled.
[http]
enabled = true

# Adds a webhook which will receive all events.
[[webhooks]]
url = "https://webhook.my-site.org"
events = ["incoming", "outgoing", "delivery", "modem_status_update"]

# Custom authorization header for the webhook.
[webhooks.headers]
Authorization = "TokenHere"
```

## Webhook Payloads

### Incoming

This event is from the carrier with incoming SMS messages. The important fields are `phone_number` and `message_content`.

```json
{
  "type": "incoming",
  "data": {
    "message_id": 9,
    "phone_number": "+447771115678",
    "message_content": "Hello! Im a message sent to the SIM!",
    "message_reference": null,
    "is_outgoing": false,
    "status": "Received",
    "created_at": null,
    "completed_at": null
  }
}
```

### Outgoing

This event is from the HTTP API, used to distribute message send responses from message producers to log consumers.

The `message_reference` is assigned by the modem or carrier.  It's not very useful externally but is used to correspond delivery reports.
It's a `u8` so wraps around to 0 once it exceeds 255.

**Available `status` initialization values:**
- **`Sent`** - The message was sent to the carrier without any errors.
- **`TemporaryFailure`** - The message failed however **it will be retried** by carrier.
- **`PermanentFailure`** - The message failed and **will not be retried** by the carrier.

```json
{
  "type": "outgoing",
  "data": {
    "message_id": 10,
    "phone_number": "+447771115678",
    "message_content": "Hi, I'm a message that's being sent from the API!",
    "message_reference": 123,
    "is_outgoing": true,
    "status": "Sent",
    "created_at": null,
    "completed_at": null
  }
}
```

### Delivery

This event is from the carrier to report the delivery status of previously sent messages. There may be a delay due to:
- **Network congestion**: Status updates can be delayed by several minutes during peak usage periods
- **Device availability**: When the recipient's phone is powered off or unreachable, status notifications will be queued until the device comes back online, up to the message's `validity_period` (maximum 72 hours).

**Field Descriptions:**

- **`report_id`** - Internal delivery report ID.
- **`message_id`** - Corresponds with `message_id` found in `outgoing` event.
- **`status`** - The [TP-Status](https://www.etsi.org/deliver/etsi_ts/123000_123099/123040/16.00.00_60/ts_123040v160000p.pdf#page=71) as `u8`.
- **`is_final`** - If no more delivery reports are expected.

```json
{
  "type": "delivery",
  "data": {
    "message_id": 10,
    "report": {
      "report_id": 7,
      "status": 0,
      "is_final": true,
      "created_at": null
    }
  }
}
```

### Modem Status Update

This event is sent from the ModemWorker when the modem serial connection has been detected as offline or when connection
is re-established. The data is the ModemStatus.

| State Name     | Description                                                               |
|----------------|---------------------------------------------------------------------------|
| `Startup`      | Only used as initial state, so only found in previous.                    |
| `Online`       | The modem serial connection is alive.                                     |
| `ShuttingDown` | The modem has sent a `SHUTTING DOWN` message, used in graceful shutdowns. |
| `Offline`      | The modem connection has closed or a timeout was detected.                |

> This status reflects the Modem Hat hardware connection, not the cellular carrier network status.

```json
{
  "type": "modem_status_update",
  "data": {
    "previous": "Online",
    "current": "ShuttingDown"
  }
}
```

## Limitations

Delivery confirmation only tracks the final message part of a multipart message, creating potential for undetected failures in earlier parts.
Sequential delivery makes partial reception unlikely but possible.

## Todo

- Support both Postgres and SQLite as database options (or just Postgres).
- Make database message storage entirely optional?
- Properly document API routes.