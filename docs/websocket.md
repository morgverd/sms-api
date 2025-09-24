# Real-time Events via WebSocket

The SMS Gateway provides real-time event streaming through WebSocket connections, allowing you to receive notifications instantly as they occur.

## Connection Details

**Base URI:** `ws://localhost:3000/ws`

> [!IMPORTANT]
> When TLS is enabled in your HTTP configuration, use `wss://` instead of `ws://` for secure WebSocket connections.

### Authentication

WebSocket connections follow the same authentication requirements as HTTP endpoints.
If `require_authentication` is enabled, include authentication headers in your connection request.

## Event Types

The following event types are available for subscription:

| Event Type             | Description                               |
|------------------------|-------------------------------------------|
| `incoming`             | New SMS message received by the modem     |
| `outgoing`             | SMS message sent from the gateway         |
| `delivery`             | Delivery status updates for sent messages |
| `modem_status_update`  | Modem connection and status changes       |
| `gnss_position_report` | GNSS location updates (if enabled)        |

> [!NOTE]
> Available events depend on your modem capabilities and configuration. Not all modems support delivery reports or GNSS.

## Event Filtering

Filter events by adding the `events` query parameter to your connection URI. Specify a comma-separated list of event types you want to receive.

### Examples

**Receive all events:**
```
ws://localhost:3000/ws
```

**Receive only message events:**
```
ws://localhost:3000/ws?events=incoming,outgoing
```

**Receive messages and delivery reports:**
```
ws://localhost:3000/ws?events=incoming,outgoing,delivery
```

## Client Examples

### JavaScript (Browser)
```javascript
const ws = new WebSocket('ws://localhost:3000/ws?events=modem_status_update');

ws.onopen = function(event) {
    console.log('Connected to SMS Gateway WebSocket');
};

ws.onmessage = function(event) {
    const data = JSON.parse(event.data);
    console.log('Received event:', data);
};

ws.onclose = function(event) {
    console.log('WebSocket connection closed');
    // Implement reconnection logic here
};

ws.onerror = function(error) {
    console.error('WebSocket error:', error);
};
```

### Python (websockets library)
```python
import asyncio
import websockets
import json

async def listen_for_events():
    uri = "ws://localhost:3000/ws?events=incoming,outgoing"
    
    async with websockets.connect(uri) as websocket:
        async for message in websocket:
            event = json.loads(message)
            print(f"Received {event['event_type']}: {event['data']}")

# Run the client
asyncio.run(listen_for_events())
```

## Configuration

WebSocket functionality is controlled by the following configuration options:

```toml
[http]
enabled = true
websocket_enabled = true  # Enable/disable WebSocket support
require_authentication = true  # Apply auth to WebSocket connections
```