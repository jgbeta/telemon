# telemon-macmon

Minimal vendored library wrapper around the MIT-licensed `macmon` 0.6.1 sampler source.

Telemon uses this only for the optional Apple Silicon `macos-macmon` exporter feature. The upstream `macmon` crate version compatible with Telemon's Clap policy is a CLI-only package, so this wrapper exposes the sampler modules as a local library without bringing in CLI dependencies.
