---
status: accepted
---
# Pure-Rust geodata readers, with WGS84 to UTM reprojection built in

The desktop app and CLI read GeoTIFF DEMs, and GeoJSON and GeoPackage vectors, with pure-Rust
readers. They reproject WGS84 input to the right UTM zone themselves. GDAL is not used, because
it would bring back the native-library install problem that ADR 0001 removes. Reprojection is
built in because an outside user's first river is likely a WGS84 trace on a geographic DEM
(Copernicus GLO-30), and refusing it would fail the first run. Other formats and other source
CRSs are refused with a message that says what to convert to.
