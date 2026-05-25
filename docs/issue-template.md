# Issue Report

## Summary

Briefly describe the problem.

## Phase and Module

Example:

- Phase: 2
- Module: P2-03 Linux hwmon collector integration

## Environment

- OS:
- Kernel:
- Architecture:
- Rust version:
- Exporter version or commit:
- Running mode:
  - cargo run
  - installed systemd service

## Config

Paste config with any sensitive values removed.

## Expected Behavior

What should have happened?

## Actual Behavior

What happened instead?

## Commands Run

```bash
cargo run -p telemon-cli -- exporter check --config config.example.yml
cargo run -p telemon-cli -- exporter discover --config config.example.yml
cargo run -p telemon-cli -- exporter print-metrics --config config.example.yml
curl http://127.0.0.1:9185/metrics
```

## Relevant Logs

Paste relevant logs here.

For systemd:

```bash
systemctl status telemon-exporter
journalctl -u telemon-exporter -n 200
```

## Metrics Output

Paste relevant `/metrics` lines here.

## Hardware Sensor Info

Paste safe output from:

```bash
find /sys/class/hwmon -maxdepth 2 -type f \( -name "name" -o -name "temp*_input" -o -name "temp*_label" -o -name "temp*_crit" -o -name "temp*_max" \) -print
```

Do not include serial numbers unless needed.

## NVIDIA GPU Info

If the issue involves `nvidia_nvml`, include:

- NVIDIA driver version, if known:
- GPU model, if visible:
- `collectors.nvidia_nvml` config:
- `cargo run -p telemon-cli -- exporter discover --config config.example.yml` output:
- Relevant `/metrics` lines containing `nvidia_nvml`, `hardware_device_info`, or `component="gpu"`:

Do not include GPU UUIDs unless the issue specifically requires them.

## Debug Bundle

Attach output from:

```bash
bash scripts/collect-debug-info.sh
```

## Notes

Any guesses, recent changes, or unusual hardware details.
