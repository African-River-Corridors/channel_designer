//! The DEM for a reach (spec 0012, T5), on the same fixtures natively and in wasm: the tile list for a
//! known box, two tiles dropped in making one DEM with no seam (the same DEM as when fetched), and a tile
//! the source withholds named and not skipped.
//!
//! Label: sourced - GLO-30 tiles are 1 degree, named by their south-west corner (Copernicus DEM product
//! handbook); the Sassandra mouth (6.08 W, 4.95 N) is in `N04_00_W007_00`.

macro_rules! t {
    ($(#[$m:meta])* fn $n:ident() $b:block) => {
        $(#[$m])*
        #[cfg_attr(not(target_arch = "wasm32"), test)]
        #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
        fn $n() $b
    };
}

use owd_format::design::dem_source::{TileId, write_geotiff};
use owd_format::geotiff::Dem;
use owd_reach::dem::{DemTile, dem_from_tiles, download_list, fetch_tiles, tiles_for_bbox};
use owd_reach::{DroppedFiles, Fetch, FetchError, ReplayFetch};

t! {
fn the_tile_list_for_a_known_box_is_the_hand_checked_one() {
    // From the sea at Sassandra town (6.08 W, 4.95 N) to north of 5 N, with no margin.
    let t = tiles_for_bbox([-6.09, 4.94, -6.07, 5.02], 0.0);
    let names: Vec<String> = t.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        [
            "Copernicus_DSM_COG_10_N04_00_W007_00_DEM",
            "Copernicus_DSM_COG_10_N05_00_W007_00_DEM"
        ]
    );
    // A 1 km margin does not add a tile here; a box over a corner takes four.
    assert_eq!(tiles_for_bbox([-6.09, 4.94, -6.07, 5.02], 1_000.0).len(), 2);
    assert_eq!(tiles_for_bbox([-3.01, 4.99, -2.99, 5.01], 0.0).len(), 4);
    assert_eq!(
        t[1].url(),
        "https://copernicus-dem-30m.s3.eu-central-1.amazonaws.com/Copernicus_DSM_COG_10_N05_00_W007_00_DEM/Copernicus_DSM_COG_10_N05_00_W007_00_DEM.tif"
    );
    let l = download_list([-6.09, 4.94, -6.07, 5.02], 0.0);
    assert_eq!(l[0].0, "Copernicus_DSM_COG_10_N04_00_W007_00_DEM.tif");
}
}

fn tile(lon: i32, lat: i32, f: impl Fn(usize, usize) -> f64) -> (DemTile, Vec<u8>) {
    let n = 36;
    let data: Vec<f64> = (0..n * n).map(|i| f(i % n, i / n)).collect();
    let dem = Dem {
        width: n,
        height: n,
        data,
        geotransform: [
            1.0 / n as f64,
            0.0,
            f64::from(lon),
            0.0,
            -1.0 / n as f64,
            f64::from(lat + 1),
        ],
        nodata: None,
        epsg: Some(4326),
    };
    (DemTile(TileId { lon, lat }), write_geotiff(&dem).unwrap())
}

t! {
fn two_tiles_dropped_in_make_one_dem_with_no_seam() {
    // A ramp that rises 1 m per pixel eastwards across both tiles: a seam would show as a jump.
    let (t0, b0) = tile(-4, 5, |c, _| c as f64);
    let (t1, b1) = tile(-3, 5, |c, _| (c + 36) as f64);
    let dropped = DroppedFiles::new()
        .with_file(&t0.file_name(), b0)
        .with_file(&t1.file_name(), b1);
    let dem = dem_from_tiles(&dropped, &[t0, t1]).unwrap();
    assert_eq!((dem.width, dem.height), (72, 36));
    for c in 0..72 {
        assert_eq!(dem.value(5, c) as f32, c as f32, "column {c}");
    }
    // The same tiles fetched give the same DEM as dropped in.
    let fetched = ReplayFetch::new()
        .with_response(&t0.url(), tile(-4, 5, |c, _| c as f64).1)
        .with_response(&t1.url(), tile(-3, 5, |c, _| (c + 36) as f64).1);
    let again = dem_from_tiles(&fetched, &[t0, t1]).unwrap();
    assert_eq!(write_geotiff(&dem).unwrap(), write_geotiff(&again).unwrap());
}
}

t! {
fn a_tile_the_source_withholds_is_named_and_not_skipped() {
    struct Sea;
    impl Fetch for Sea {
        fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            Err(FetchError::NotFound(url.to_string()))
        }
        fn get_range(&self, url: &str, _: u64, _: u64) -> Result<Vec<u8>, FetchError> {
            Err(FetchError::NotFound(url.to_string()))
        }
    }
    let t = tiles_for_bbox([-6.09, 4.94, -6.07, 4.96], 0.0);
    let f = fetch_tiles(&Sea, &t).unwrap();
    assert_eq!(f.withheld, t);
    let e = dem_from_tiles(&Sea, &t).unwrap_err().to_string();
    assert!(
        e.contains("Copernicus_DSM_COG_10_N04_00_W007_00_DEM") && e.contains("sea"),
        "{e}"
    );
    // Dropped files that are missing say so too (a different failure: the user has not given it).
    assert!(dem_from_tiles(&DroppedFiles::new(), &t).is_err());
}
}
