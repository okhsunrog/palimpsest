set shell := ["bash", "-euo", "pipefail", "-c"]

# QEMU host-forward port. Distinct from archinstall_zfs (2222).
SSH_PORT := "2226"
# archzfs test ISO comes from sibling archinstall_zfs/gen_iso/out/.
# Build with `just iso-test` from that repo; the test variant
# (filename contains "pre-testing") has root login with empty password.
ISO_GLOB := env_var('HOME') + "/code/archinstall_zfs/gen_iso/out/archzfs-*-testing-*.iso"
QEMU_LOG := "/tmp/palimpsest-qemu.log"
QEMU_PIDFILE := "/tmp/palimpsest-qemu.pid"
UEFI_VARS := "/tmp/palimpsest-OVMF_VARS.fd"
SSH_TARGET := "root@localhost:" + SSH_PORT
SSH_OPTS := "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o PreferredAuthentications=password -o PubkeyAuthentication=no"

# ─── Help ──────────────────────────────────────────────

# Show this list
default:
    @just --list

# ─── Cargo ─────────────────────────────────────────────

check:
    cargo check --all-targets

test:
    cargo test --lib --tests

lint:
    cargo clippy --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# ─── VM lifecycle ──────────────────────────────────────

# Boot the archzfs test ISO in a backgrounded headless QEMU. Requires
# `qemu-system-x86_64`, `edk2-ovmf`, `sshpass`. The VM listens on
# localhost:{{SSH_PORT}} for SSH (root, empty password).
vm-up:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -f {{QEMU_PIDFILE}} ]] && kill -0 "$(cat {{QEMU_PIDFILE}})" 2>/dev/null; then
        echo "VM already running (pid $(cat {{QEMU_PIDFILE}}))"
        exit 0
    fi
    iso=$(ls -1t {{ISO_GLOB}} 2>/dev/null | head -n1 || true)
    if [[ -z "$iso" ]]; then
        echo "ERROR: no archzfs test ISO found matching {{ISO_GLOB}}"
        echo "  Build it: cd ~/code/archinstall_zfs && just iso-test"
        exit 1
    fi
    cp -f /usr/share/edk2/x64/OVMF_VARS.4m.fd {{UEFI_VARS}}
    echo "Booting $(basename "$iso") on port {{SSH_PORT}}..."
    nohup qemu-system-x86_64 \
        -enable-kvm -cpu host -m 2048 -smp 2 \
        -boot order=d -display none \
        -net nic -net user,hostfwd=tcp::{{SSH_PORT}}-:22 \
        -machine type=q35,smm=on,accel=kvm,usb=on \
        -global ICH9-LPC.disable_s3=1 -no-reboot \
        -drive if=pflash,format=raw,unit=0,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd,read-only=on \
        -drive if=pflash,format=raw,unit=1,file={{UEFI_VARS}} \
        -cdrom "$iso" \
        > {{QEMU_LOG}} 2>&1 &
    echo $! > {{QEMU_PIDFILE}}
    echo "QEMU pid $(cat {{QEMU_PIDFILE}}) — log {{QEMU_LOG}}"
    echo "Waiting for SSH on port {{SSH_PORT}}..."
    for i in $(seq 1 60); do
        if sshpass -p "" ssh {{SSH_OPTS}} -p {{SSH_PORT}} root@localhost "echo ready" >/dev/null 2>&1; then
            echo "VM ready."
            exit 0
        fi
        sleep 2
    done
    echo "ERROR: VM did not become SSH-ready within 120s. Last log:"
    tail -n 20 {{QEMU_LOG}}
    exit 1

# Power off the VM cleanly.
vm-down:
    #!/usr/bin/env bash
    set -uo pipefail
    if [[ -f {{QEMU_PIDFILE}} ]]; then
        sshpass -p "" ssh {{SSH_OPTS}} -p {{SSH_PORT}} root@localhost "poweroff" >/dev/null 2>&1 || true
        sleep 1
        kill "$(cat {{QEMU_PIDFILE}})" 2>/dev/null || true
        rm -f {{QEMU_PIDFILE}}
    fi
    echo "VM stopped."

# Open an interactive SSH session to the running VM.
vm-ssh:
    sshpass -p "" ssh {{SSH_OPTS}} -p {{SSH_PORT}} root@localhost

# Show the QEMU console log.
vm-log:
    @tail -n 80 {{QEMU_LOG}}

# Sweep stale palimpsest_test_* pools and backing files inside the VM. Run
# after a panicked test to clean up stragglers.
test-cleanup:
    @sshpass -p "" ssh {{SSH_OPTS}} -p {{SSH_PORT}} root@localhost "\
        zpool list -H -o name 2>/dev/null | grep '^palimpsest_test_' | \
        xargs -rn1 -I{} bash -c 'zpool destroy -f {} 2>/dev/null || true'; \
        rm -f /tmp/palimpsest_test_*.img; \
        rm -rf /tmp/palimpsest_test_*_root"

# ─── Integration tests ─────────────────────────────────

# Run the integration test suite against an already-running VM. Use
# `just test-vm` for a one-shot boot+test+shutdown cycle.
test-integration:
    PALIMPSEST_SSH_TARGET={{SSH_TARGET}} \
    PALIMPSEST_SSH_PASSWORD="" \
        cargo test --tests --features integration -- --test-threads=1

# Boot VM, run integration tests, tear down. Slowest path; use for CI / a
# clean check. For inner-loop dev: `just vm-up` once, then `just test-integration`
# many times.
test-vm: vm-up
    just test-integration
    just vm-down
