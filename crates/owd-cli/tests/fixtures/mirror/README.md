# A recorded mirror for `owd fetch-dem` tests

`Copernicus_DSM_COG_10_N05_00_W003_00_DEM/Copernicus_DSM_COG_10_N05_00_W003_00_DEM.tif` is the 80 x 80
pixel clip of the real Copernicus GLO-30 tile N05 W003 that is also a fixture of the GeoTIFF reader
(`crates/owd-format/tests/fixtures/dem/README.md` says where it came from). It stands in for the tile
in tests, served from a folder laid out like the public bucket, so CI never uses the network.

Produced using Copernicus WorldDEM-30 (c) DLR e.V. 2010-2014 and (c) Airbus Defence and Space GmbH
2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved.
