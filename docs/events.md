# Events

Events are emitted by the application, and can be received via a WebSocket connection or Webhook requests (HTTP).
The payloads are the same for both connection types.

## Incoming

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

## Outgoing

This event is from the HTTP API, used to distribute message send responses from message producers to log consumers.

The `message_reference` is assigned by the modem or carrier.  It's not very useful externally but is used to correspond delivery reports.
It's a `u8` so wraps around to 0 once it exceeds 255.

**Available `status` initialization values:**

| Status             | Description                                                    |
|--------------------|----------------------------------------------------------------|
| `Sent`             | The message was sent to the carrier without any errors.        |
| `TemporaryFailure` | The message failed however **it will be retried** by carrier.  |
| `PermanentFailure` | The message failed and **will not be retried** by the carrier. |

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

## Delivery

This event is from the carrier to report the delivery status of previously sent messages. There may be a delay due to:
- **Network congestion**: Status updates can be delayed by several minutes during peak usage periods
- **Device availability**: When the recipient's phone is powered off or unreachable, status notifications will be queued until the device comes back online, up to the message's `validity_period` (maximum 72 hours).

| Field        | Description                                                                                                                   |
|--------------|-------------------------------------------------------------------------------------------------------------------------------|
| `report_id`  | Internal delivery report ID.                                                                                                  |
| `message_id` | Corresponds with `message_id` found in `outgoing` event.                                                                      |
| `status`     | The [TP-Status](https://www.etsi.org/deliver/etsi_ts/123000_123099/123040/16.00.00_60/ts_123040v160000p.pdf#page=71) as `u8`. |
| `is_final`   | If no more delivery reports are expected.                                                                                     |

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

## Modem Status Update

This event is sent from the ModemWorker when the modem serial connection has been detected as offline or when connection
is re-established. The data is the ModemStatus.

| State Name     | Description                                                               |
|----------------|---------------------------------------------------------------------------|
| `Startup`      | Only used as initial state, so only found in previous.                    |
| `Online`       | The modem serial connection is alive.                                     |
| `ShuttingDown` | The modem has sent a `SHUTTING DOWN` message, used in graceful shutdowns. |
| `Offline`      | The modem connection has closed or a timeout was detected.                |

> [!NOTE]
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

## GNSS Position Report

This event is sent from the GNSS module when `modem.gnss_enabled` is enabled. It broadcasts GPS position data (longitude, latitude, speed, etc.) at intervals specified by `modem.gnss_report_interval` (defaults to `0`, which disables reporting).

> [!NOTE]
> This event is only emitted when `modem.gnss_enabled` is `true` and `modem.gnss_report_interval` is greater than `0`.

```json
{
  "type": "gnss_position_report",
  "data": {
    "run_status": true,
    "fix_status": true,
    "utc_time": 4294967295,
    "latitude": 35.126122,
    "longitude": -106.536530,
    "msl_altitude": 30.250,
    "ground_speed": 0.0,
    "ground_course": 16.2,
    "fix_mode": "Fix3D",
    "hdop": 0.7,
    "pdop": 0.9,
    "vdop": 0.6,
    "gps_in_view": 13,
    "gnss_used": 13,
    "glonass_in_view": 10
  }
}
```
