# SMS Server

A powerful, self-hosted SMS gateway that enables you to send and receive text messages using your own cellular hardware. Built with security and flexibility in mind.

## Features

### 🔐 **Security First**
- **End-to-end encryption** for all SMS message content stored in the database
- Optional token-based authentication for HTTP API and WebSocket connections
- TLS/HTTPS support for secure communications

### 📡 **Multiple Integration Options**
- **HTTP REST API** for programmatic SMS sending and database access
- **Real-time WebSocket** connections for instant low-overhead event notifications
- **HTTP webhooks** to push events to external systems
- Comprehensive event filtering and routing

### 📱 **Advanced SMS Features**
- Automatic handling of multipart SMS messages
- SMS delivery report tracking with status updates
- Support for both incoming and outgoing message management
- International phone number format handling

### 🛰️ **Location Services**
- Built-in GNSS/GPS location tracking (configurable)
- Real-time position reporting via events
- Location data integration with SMS workflows

## Getting Started

1. **Hardware Setup**: Connect your GSM modem to your device
2. **Configuration**: Create a `config.toml` file (see [Configuration Guide](docs/configuration.md))
3. **Launch**: Start the gateway and begin sending/receiving SMS messages

## Documentation

| Document                                        | Description                                        |
|-------------------------------------------------|----------------------------------------------------|
| [📋 Configuration Guide](docs/configuration.md) | Complete configuration reference with examples     |
| [📡 Event Types](docs/events.md)                | Available events received via WebSocket or Webhook |
| [🔗 HTTP API Reference](docs/http.md)           | REST API endpoints for SMS operations              |
| [⚡ WebSocket Guide](docs/websocket.md)          | Real-time event streaming setup                    |

## Examples

### [💬 ChatGPT SMS Bot](./examples/chatgpt-sms)

An intelligent SMS responder that integrates with OpenAI's ChatGPT API. Receives incoming messages via webhooks, generates contextual replies using conversation history, and responds automatically. Features conversation memory and customizable response templates.

> [!NOTE]
> Possibly the first ChatGPT SMS implementation running directly through cellular modem hardware!

### [🗺️ Real-time GNSS Viewer](./examples/gnss-viewer)

A web-based GPS tracking dashboard that connects via WebSocket to display live position updates. Monitor location accuracy, track movement patterns, and analyze GPS performance in real-time. Accessible from any networked device with a modern web browser.

## Hardware Requirements

You'll need some form of GSM modem that allows for serial connection.
I use (and this project has only been tested with) a [Waveshare GSM Pi Hat](https://www.waveshare.com/gsm-gprs-gnss-hat.htm) on a Raspberry Pi.

> [!TIP]
> Many SIM cards require carrier-specific APN configuration and network registration before SMS functionality becomes available.

## Known Limitations

- **Delivery Confirmation Scope**: Only the final segment of multipart SMS messages receives delivery confirmation tracking, which may mask delivery failures in earlier message parts.
- **Sequential Processing**: Messages are processed sequentially, which ensures reliability but may impact throughput for high-volume scenarios.
