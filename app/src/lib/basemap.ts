// The basemap. Display constants only. Option colours come from the tokens (theme.ts).

/** The only basemap: EOX Sentinel-2 cloudless 2016. The 2018-2025 layers are not offered (licence). */
export const BASEMAP_TILES = "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless_3857/default/g/{z}/{y}/{x}.jpg";

/** Always shown on the map, as the licence requires. */
export const ATTRIBUTION =
  "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)";
