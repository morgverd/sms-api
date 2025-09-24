# Configuration Documentation

This document describes the configuration file format and options for the SMS/Modem Gateway application. The configuration uses TOML format and is typically stored as `config.toml` in the application root directory.

## Table of Contents

- [Database Configuration](#database-configuration)
- [Modem Configuration](#modem-configuration)
- [HTTP Server Configuration](#http-server-configuration)
- [TLS Configuration](#tls-configuration)
- [Webhook Configuration](#webhook-configuration)
- [Sentry Configuration](#sentry-configuration-optional)
- [Complete Example](#complete-example)

## Database Configuration

The database section configures the connection to your database and encryption settings.

### Required Fields

| Field            | Type   | Description                            |
|------------------|--------|----------------------------------------|
| `database_url`   | String | Database connection URL.               |
| `encryption_key` | String | Base64-encoded 32-byte encryption key. |

### Example

```toml
[database]
database_url = "postgresql://user:password@localhost:5432/sms_gateway"
encryption_key = "SGVsbG8gV29ybGQhIFRoaXMgaXMgYSAzMiBieXRlIGtleQ=="
```

> [!TIP]
> Generate a secure encryption key using: `openssl rand -base64 32`

## Modem Configuration

The modem section configures the cellular modem connection and behavior.

### Fields

| Field                     | Type    | Default        | Description                                    |
|---------------------------|---------|----------------|------------------------------------------------|
| `device`                  | String  | `"/dev/ttyS0"` | Serial device path for the modem               |
| `baud`                    | Number  | `115200`       | Serial baud rate                               |
| `gnss_enabled`            | Boolean | `false`        | Enable GPS/GNSS functionality                  |
| `gnss_report_interval`    | Number  | `0`            | GNSS report interval in seconds (0 = disabled) |
| `gpio_power_pin`          | Boolean | `false`        | Use GPIO power pin control                     |
| `gpio_repower`            | Boolean | `true`         | Allow GPIO repower operations                  |
| `cmd_channel_buffer_size` | Number  | `32`           | Command channel buffer size                    |
| `read_buffer_size`        | Number  | `4096`         | Read buffer size in bytes                      |
| `line_buffer_size`        | Number  | `4096`         | Line buffer size in bytes                      |

### Example

```toml
[modem]
device = "/dev/ttyUSB0"
baud = 115200
gnss_enabled = true
gnss_report_interval = 30
gpio_power_pin = true
gpio_repower = true
cmd_channel_buffer_size = 64
read_buffer_size = 8192
line_buffer_size = 8192
```

### Notes

- All fields are optional and will use defaults if not specified.
- GNSS reporting interval of 0 disables periodic reports.

## HTTP Server Configuration

The HTTP section configures the web server for REST API and WebSocket connections.

### Fields

| Field                            | Type                            | Default            | Description                               |
|----------------------------------|---------------------------------|--------------------|-------------------------------------------|
| `enabled`                        | Boolean                         | `false`            | Enable HTTP server                        |
| `address`                        | String                          | `"127.0.0.1:3000"` | Server bind address and port              |
| `send_international_format_only` | Boolean                         | `true`             | Only send numbers in international format |
| `require_authentication`         | Boolean                         | `true`             | Require authentication for API access     |
| `websocket_enabled`              | Boolean                         | `true`             | Enable WebSocket support                  |
| `phone_number`                   | String                          | `null`             | Default phone number for the modem        |
| `tls`                            | [TLSConfig](#tls-configuration) | `null`             | TLS configuration (see below)             |

### Example

```toml
[http]
enabled = true
address = "0.0.0.0:8080"
send_international_format_only = true
require_authentication = true
websocket_enabled = true
phone_number = "+1234567890"
```

### Notes

- Set `address` to `0.0.0.0:port` to accept connections from any IP.
- Use `127.0.0.1:port` for localhost-only access.
- Phone number should be in international format (starting with +).

## TLS Configuration

TLS configuration is a subsection of the HTTP configuration that enables HTTPS.

### Fields

| Field       | Type   | Description                  |
|-------------|--------|------------------------------|
| `cert_path` | String | Path to TLS certificate file |
| `key_path`  | String | Path to TLS private key file |

### Example

```toml
[http]
enabled = true
address = "0.0.0.0:8443"

[http.tls]
cert_path = "/path/to/certificate.crt"
key_path = "/path/to/private.key"
```

### Notes

- Both certificate and key files must exist and be readable.
- Use full paths to certificate files.
- The application will validate file existence at startup.

## Webhook Configuration

Webhooks allow the application to send HTTP requests when specific events occur.

### Fields

| Field             | Type   | Default        | Description                          |
|-------------------|--------|----------------|--------------------------------------|
| `url`             | String | -              | Webhook endpoint URL                 |
| `expected_status` | Number | `null`         | Expected HTTP status code (optional) |
| `events`          | Array  | `["incoming"]` | List of events to trigger webhook    |
| `headers`         | Object | `null`         | Custom HTTP headers                  |
| `certificate`     | String | `null`         | Path to custom CA certificate        |

### Available Events

- `IncomingMessage` - New SMS message received
- (Additional events depend on your `EventType` enum)

### Example

```toml
[[webhooks]]
url = "https://api.example.com/sms-webhook"
events = ["incoming", "outgoing"]

[webhooks.headers]
Authorization = "Bearer your-token-here"

[[webhooks]]
url = "https://internal.company.com/notifications"
expected_status = 204
events = ["incoming"]
certificate = "/path/to/internal-ca.crt"
```

### Notes

- Multiple webhooks can be configured using `[[webhooks]]` array syntax.
- If `expected_status` is not specified, any 2xx status is considered success.
- Custom certificates are useful for internal/self-signed endpoints.
- Headers are optional and can include authentication tokens.

## Sentry Configuration (Optional)

Sentry integration provides error tracking and performance monitoring. This section is only available when compiled with the `sentry` feature.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `dsn` | String | - | Sentry Data Source Name |
| `environment` | String | `null` | Environment name (e.g., "production") |
| `server_name` | String | `null` | Server name for event tagging |
| `debug` | Boolean | `false` | Enable Sentry debug mode |
| `send_default_pii` | Boolean | `true` | Send personally identifiable information |

### Example

```toml
[sentry]
dsn = "https://your-dsn@sentry.io/project-id"
environment = "production"
server_name = "sms-gateway-01"
debug = false
send_default_pii = false
```

### Notes

- This section is only processed when the application is built with Sentry support.
- DSN can be found in your Sentry project settings.
- Set `send_default_pii = false` for privacy-sensitive deployments.

## Complete Example

Here's a complete configuration file example:

```toml
# Database configuration
[database]
database_url = "postgresql://sms_user:secure_password@localhost:5432/sms_gateway"
encryption_key = "SGVsbG8gV29ybGQhIFRoaXMgaXMgYSAzMiBieXRlIGtleQ=="

# Modem configuration
[modem]
device = "/dev/ttyUSB0"
baud = 115200
gnss_enabled = true
gnss_report_interval = 60
gpio_power_pin = true
gpio_repower = true
cmd_channel_buffer_size = 64
read_buffer_size = 8192
line_buffer_size = 8192

# HTTP server configuration
[http]
enabled = true
address = "0.0.0.0:8080"
send_international_format_only = true
require_authentication = true
websocket_enabled = true
phone_number = "+1234567890"

# TLS configuration (HTTPS)
[http.tls]
cert_path = "/etc/ssl/certs/sms-gateway.crt"
key_path = "/etc/ssl/private/sms-gateway.key"

# Webhook configurations
[[webhooks]]
url = "https://api.myservice.com/sms-received"
events = ["incoming"]

[[webhooks]]
url = "https://internal.company.com/alerts"
expected_status = 204
events = ["incoming"]
certificate = "/etc/ssl/certs/company-ca.crt"

# Sentry error tracking (optional)
[sentry]
dsn = "https://your-key@sentry.io/project-id"
environment = "production"
server_name = "gateway-prod-01"
debug = false
send_default_pii = false
```

## Configuration File Loading

The application looks for the configuration file in the following order:

1. Path specified via command line argument. Eg: `sms-server -c config.toml`
2. `config.toml` in the current working directory.

If the configuration file cannot be found or parsed, the application will exit with an error message.

## Security Considerations

- Store the configuration file securely with appropriate file permissions.
- Use strong, randomly generated encryption keys.
- Regularly rotate encryption keys and authentication tokens.
- Use TLS for all webhook endpoints when possible.
