# WebSocket GNSS Viewer

A very simple webpage that allows for live monitoring of GNSS position reports via it's websocket connection.
This was created, so I could monitor these values on my laptop while testing tracking accuracy in a moving car.

![GNSS Position Tracker View](/.github/assets/gnss-position-tracker.png)

## Config

The following config options must be set:
```toml
[modem]

# This determines how often the GNSS data should update (in seconds).
gnss_report_interval = 5
gnss_enabled = true

[http]
enabled = true
websocket_enabled = true
require_authentication = false
```
