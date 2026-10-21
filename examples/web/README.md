# Web fixtures

The files the web parity check (`tools/web_parity.mjs`) and the web app's example run on (spec 0011).

- `river-dem.tif`: the terrain of a short synthetic river, as a deflated float32 GeoTIFF in UTM zone 30N.
- `river-example.owd`: sizing, rolling circle and volumes, all fresh. The web app's "Open the example" opens it.
- `river-rolling-circle.owd`, `river-reroute.owd`: every stage has run, then the draught changed, so the
  later stages are stale and a run must do them again (the pool search, the re-route, volumes).

All three documents name `river-dem.tif` by relative path and SHA-256. Rebuild them, after a change that
moves results (they are committed, so nothing else changes them):

```bash
cargo test --release -p owd-cli --test web_fixtures -- --ignored write_the_web_fixtures
```
