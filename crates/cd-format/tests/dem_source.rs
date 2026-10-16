//! Tile names, joining tiles, and writing a DEM as a GeoTIFF (what `cdesign fetch-dem` plans with).
//!
//! Label: property

use cd_format::design::dem_source::{TileId, mosaic, tiles_for, write_geotiff};
use cd_format::geotiff::{Dem, read_bytes};

#[test]
fn tiles_are_named_by_their_south_west_corner_like_the_public_bucket() {
    assert_eq!(
        TileId { lon: -3, lat: 5 }.name(),
        "Copernicus_DSM_COG_10_N05_00_W003_00_DEM"
    );
    assert_eq!(
        TileId { lon: 36, lat: -2 }.name(),
        "Copernicus_DSM_COG_10_S02_00_E036_00_DEM"
    );
    assert_eq!(
        TileId { lon: -3, lat: 5 }.url("https://example.test/bucket/"),
        "https://example.test/bucket/Copernicus_DSM_COG_10_N05_00_W003_00_DEM/Copernicus_DSM_COG_10_N05_00_W003_00_DEM.tif"
    );
}

#[test]
fn a_box_needs_every_tile_it_touches() {
    // Inside one tile.
    assert_eq!(
        tiles_for((-2.6, 5.4, -2.5, 5.6)),
        vec![TileId { lon: -3, lat: 5 }]
    );
    // Across a meridian and a parallel: four tiles. West of the meridian is the lower tile index.
    let t = tiles_for((-3.1, 4.9, -2.9, 5.1));
    assert_eq!(t.len(), 4);
    assert!(t.contains(&TileId { lon: -4, lat: 4 }) && t.contains(&TileId { lon: -3, lat: 5 }));
    // South of the equator, -0.5 is in the tile whose corner is -1.
    assert_eq!(
        tiles_for((10.2, -0.5, 10.3, -0.4)),
        vec![TileId { lon: 10, lat: -1 }]
    );
}

fn tile(left: f64, top: f64, fill: f64) -> Dem {
    Dem {
        width: 4,
        height: 3,
        data: (0..12).map(|i| fill + i as f64).collect(),
        geotransform: [0.5, 0.0, left, 0.0, -0.5, top],
        nodata: Some(-9999.0),
        epsg: Some(4326),
    }
}

#[test]
fn tiles_on_one_grid_are_joined_with_every_pixel_kept() {
    let (a, b) = (tile(10.0, 5.0, 0.0), tile(12.0, 5.0, 100.0));
    let (c, d) = (tile(10.0, 3.5, 200.0), tile(12.0, 3.5, 300.0));
    let m = mosaic(&[d.clone(), a.clone(), c.clone(), b.clone()]).unwrap();
    assert_eq!((m.width, m.height), (8, 6));
    assert_eq!(m.geotransform, [0.5, 0.0, 10.0, 0.0, -0.5, 5.0]);
    for (t, c0, r0) in [(&a, 0, 0), (&b, 4, 0), (&c, 0, 3), (&d, 4, 3)] {
        for r in 0..3 {
            for col in 0..4 {
                assert_eq!(m.value(r0 + r, c0 + col), t.value(r, col));
            }
        }
    }
}

#[test]
fn tiles_that_do_not_share_a_grid_or_leave_a_gap_are_refused() {
    let a = tile(10.0, 5.0, 0.0);
    let mut coarse = tile(12.0, 5.0, 0.0);
    coarse.geotransform[0] = 0.75;
    assert!(mosaic(&[a.clone(), coarse]).unwrap_err().contains("grid"));
    let far = tile(20.0, 5.0, 0.0);
    assert!(mosaic(&[a.clone(), far]).unwrap_err().contains("rectangle"));
    let same = tile(10.0, 5.0, 50.0);
    assert!(mosaic(&[a, same]).is_err());
}

#[test]
fn a_written_geotiff_reads_back_as_the_same_dem() {
    let mut d = tile(-2.5, 5.5, 80.25);
    d.data[3] = -9999.0; // a nodata pixel
    let bytes = write_geotiff(&d).unwrap();
    let back = read_bytes(&bytes).unwrap();
    assert_eq!((back.width, back.height, back.epsg), (4, 3, Some(4326)));
    assert_eq!(back.geotransform, d.geotransform);
    assert_eq!(back.nodata, Some(-9999.0));
    for r in 0..3 {
        for c in 0..4 {
            let (x, y) = (d.value(r, c), back.value(r, c));
            assert!(
                (x.is_nan() && y.is_nan()) || x == y,
                "({r},{c}): {x} vs {y}"
            );
        }
    }
    // A projected DEM too.
    let mut p = tile(500_000.0, 600_000.0, 1.0);
    p.epsg = Some(32630);
    assert_eq!(
        read_bytes(&write_geotiff(&p).unwrap()).unwrap().epsg,
        Some(32630)
    );
}
