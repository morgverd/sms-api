#!/bin/bash
cargo build --target aarch64-unknown-linux-gnu
rsync -avz --delete ./target/aarch64-unknown-linux-gnu/debug/ pi@192.168.1.20:/home/pi/sms-rs/