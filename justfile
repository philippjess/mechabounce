# Format check, lints, tests
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo nextest run
    cargo test --doc

build:
    cargo build --release

# Filter one keyboard in the foreground, e.g. `just run usb-Foo-event-kbd -t 40`
run keyboard *args: build
    sudo target/release/mechabounce --keyboard {{keyboard}} {{args}}

# End-to-end test through the kernel with a fake keyboard (asks for sudo)
e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    bin=$(cargo test --test it --no-run --message-format=json | jq -r 'select(.profile.test == true and .target.name == "it") | .executable')
    sudo "$bin" --ignored e2e::

# Install the binary, the systemd template and the udev rule (asks for sudo)
install: build
    sudo install -Dm755 target/release/mechabounce /usr/local/bin/mechabounce
    sudo install -Dm644 dist/mechabounce@.service /etc/systemd/system/mechabounce@.service
    sudo install -Dm644 dist/70-mechabounce.rules /etc/udev/rules.d/70-mechabounce.rules
    test -e /etc/mechabounce.conf || sudo install -Dm644 dist/mechabounce.conf /etc/mechabounce.conf
    sudo systemctl daemon-reload
    sudo udevadm control --reload
    sudo udevadm trigger --action=change --subsystem-match=input --property-match=ID_INPUT_KEYBOARD=1
