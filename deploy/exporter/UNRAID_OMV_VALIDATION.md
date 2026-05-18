# Unraid And OMV Docker Exporter Validation

Use this flow to compare the native Unraid bootstrap exporter against the
Docker exporter on the same host. The Docker test uses port `9187` and separate
state so it does not overwrite the native `install.sh` setup on port `9185`.

Run commands from the repository root on the Unraid or OMV host. The target host
does not need Cargo; Docker builds the Rust binary inside the image build.

## 1. Capture The Native Baseline

If the native exporter is running through Unraid User Scripts, capture its
metrics first:

```bash
curl -s http://127.0.0.1:9185/metrics > /tmp/telemon-native.metrics
grep 'telemon_temperature_celsius.*source="linux_hwmon"' /tmp/telemon-native.metrics
grep -c 'telemon_temperature_celsius.*source="linux_hwmon"' /tmp/telemon-native.metrics
```

This is the baseline the Docker exporter should try to match. On your tested
Unraid host this should include CPU package/core temperatures and storage
composite temperatures.

## 2. Confirm Host Sensor Visibility

Check that the host itself exposes readable hwmon files:

```bash
find -L /sys/class/hwmon -maxdepth 2 -type f \
  \( -name 'name' -o -name 'temp*_input' -o -name 'temp*_label' -o -name 'temp*_max' -o -name 'temp*_crit' \) \
  -print -exec cat {} \; 2>/dev/null | head -200
```

If this command shows no `temp*_input` files, Docker cannot expose them either.
If native metrics exist but this command is empty, run it directly in the same
Unraid terminal session used by User Scripts and check shell quoting.

## 3. Start The Docker Test Exporter

Unraid defaults to `/boot/config/plugins/telemon-docker` for Docker test
config and UUID state:

```bash
mkdir -p /boot/config/plugins/telemon-docker

export TELEMON_REGISTRY_SERVER=registry.example.local:9186
export TELEMON_ENROLLMENT_TOKEN=change-me
export TELEMON_USER_NAME=example-user
export TELEMON_DEVICE_NAME=unraid-docker
export TELEMON_ADVERTISED_ADDR=<unraid-lan-ip>

docker compose -f deploy/exporter/docker-compose.unraid-test.yml build
docker compose -f deploy/exporter/docker-compose.unraid-test.yml up -d
```

For OMV, use an OMV-friendly persistent config path before starting:

```bash
mkdir -p /srv/telemon/exporter

export TELEMON_DOCKER_CONFIG_DIR=/srv/telemon/exporter
export TELEMON_REGISTRY_SERVER=<server-ip>:9186
export TELEMON_ENROLLMENT_TOKEN=<token>
export TELEMON_USER_NAME=<user-label>
export TELEMON_DEVICE_NAME=omv-docker
export TELEMON_ADVERTISED_ADDR=<omv-lan-ip>

docker compose -f deploy/exporter/docker-compose.unraid-test.yml build
docker compose -f deploy/exporter/docker-compose.unraid-test.yml up -d
```

The test compose file intentionally uses:

```text
TELEMON_LISTEN=0.0.0.0:9187
TELEMON_SCRAPE_PORT=9187
TELEMON_FAKE_ENABLED=false
TELEMON_HWMON_ROOT=/host/sys/class/hwmon
TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN=true
/sys -> /host/sys:ro
```

## 4. Inspect Docker Visibility

Confirm the generated config points at the host sysfs mount:

```bash
docker exec telemon-exporter-docker-test sh -lc 'sed -n "1,180p" /config/generated-exporter.yml'
```

Confirm the container can see host hwmon files:

```bash
docker exec telemon-exporter-docker-test sh -lc \
  'find -L /host/sys/class/hwmon -maxdepth 2 -type f \( -name "name" -o -name "temp*_input" -o -name "temp*_label" -o -name "temp*_max" -o -name "temp*_crit" \) -print -exec cat {} \; 2>/dev/null | head -200'
```

## 5. Capture Docker Metrics

```bash
curl -s http://127.0.0.1:9187/metrics > /tmp/telemon-docker.metrics
grep 'telemon_temperature_celsius.*source="linux_hwmon"' /tmp/telemon-docker.metrics
grep -c 'telemon_temperature_celsius.*source="linux_hwmon"' /tmp/telemon-docker.metrics
grep 'source="fake"' /tmp/telemon-docker.metrics
```

The fake grep should return no lines. If fake metrics appear, the Docker test
is not using the validation compose file or the generated config was not
updated.

Check registration and Prometheus service discovery from the monitoring server:

```bash
curl http://<server-ip>:9186/prometheus/sd
curl http://<server-ip>:9186/prometheus/sd/15s
curl http://<unraid-or-omv-lan-ip>:9187/metrics
curl http://<unraid-or-omv-lan-ip>:9187/metrics/static
```

The Docker target should appear with the Docker device name and port `9187`.

## 6. Interpret Results

Host has temp files, but the container cannot see them:

```text
Docker mount/template issue. Verify /sys is mounted to /host/sys read-only.
Do not mount only /sys/class/hwmon.
```

Container sees temp files, but Docker emits zero `linux_hwmon` temperatures:

```text
Rust collector discovery, filtering, or classification issue.
Next likely fix: add hwmon discovery diagnostics and improve classifier coverage.
```

Docker emits only `component="unknown"` sensors:

```text
Host visibility is working. Improve chip classification, but the Docker mount is no longer the blocker.
```

Docker emits CPU/storage metrics similar to native:

```text
Docker hwmon path is good. Keep the native Unraid path as baseline and use Docker for OMV/container-first installs.
```

NVIDIA status is separate. `nvidia_nvml supported=0` only means NVML is not
available to the exporter; it does not affect Linux hwmon temperature metrics.

## 7. Stop The Docker Test

```bash
docker compose -f deploy/exporter/docker-compose.unraid-test.yml down
```

The Docker UUID and generated config remain in the configured `/config`
directory so repeated tests keep the same Docker device identity.

## 8. Move To Production Docker

After Docker matches the native baseline, stop the test exporter and switch to
the production compose file on port `9185`:

```bash
docker compose -f deploy/exporter/docker-compose.unraid-test.yml down
docker compose -f deploy/exporter/docker-compose.production.yml up -d
```

Do this only after stopping any native exporter that is already using `9185`.
