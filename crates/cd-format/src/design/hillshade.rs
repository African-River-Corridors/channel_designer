//! The offline map fallback (spec 0003): a hillshade of the design's DEM, drawn once, as a PNG
//! the map lays over the vector layers when the basemap tiles cannot load.
//!
//! The picture is cut on a grid that is even in longitude and in Web Mercator y, which is the
//! grid a map image source stretches between its four corners. Each pixel is sampled from the DEM
//! where it lies, so the hillshade lines up with the vector layers however the DEM is projected.

use super::crs::Crs;
use super::session::DesignError;
use crate::geotiff::Dem;

/// The longest side of the picture, in pixels.
const MAX_SIDE: f64 = 1024.0;
/// Earth radius of the spherical Web Mercator model, in metres.
const R: f64 = 6_378_137.0;

/// A hillshade picture and where it goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Hillshade {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Top-left, top-right, bottom-right and bottom-left as `[lon, lat]` degrees: the order a map
    /// image source takes.
    pub corners: [[f64; 2]; 4],
}

fn mercator_y(lat_deg: f64) -> f64 {
    libm::log(libm::tan(
        core::f64::consts::FRAC_PI_4 + lat_deg.to_radians() / 2.0,
    ))
}

fn lat_of(y: f64) -> f64 {
    (2.0 * libm::atan(libm::exp(y)) - core::f64::consts::FRAC_PI_2).to_degrees()
}

/// A hillshade of `dem`, lit from the north-west at 45 degrees. Nodata and the ring of pixels
/// around it are transparent.
pub fn hillshade(dem: &Dem) -> Result<Hillshade, DesignError> {
    let crs = dem
        .epsg
        .ok_or_else(|| DesignError::Input("the DEM does not name its CRS".into()))
        .and_then(|e| Crs::from_epsg(e).map_err(DesignError::Input))?;
    let (x0, y0, x1, y1) = dem.bounds();
    let corners_dem = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let corners_ll = match crs {
        Crs::Wgs84 => corners_dem.to_vec(),
        Crs::Utm(u) => u.inverse_all(&corners_dem).map_err(DesignError::Input)?,
    };
    let fold = |f: fn(f64, f64) -> f64, i: usize, init: f64| {
        corners_ll.iter().fold(init, |a, c| f(a, c[i]))
    };
    let (west, east) = (
        fold(f64::min, 0, f64::INFINITY),
        fold(f64::max, 0, f64::NEG_INFINITY),
    );
    let (south, north) = (
        fold(f64::min, 1, f64::INFINITY),
        fold(f64::max, 1, f64::NEG_INFINITY),
    );
    let (my_s, my_n) = (mercator_y(south), mercator_y(north));
    let (dlon, dmy) = (east - west, my_n - my_s);
    if !(dlon > 0.0 && dmy > 0.0) {
        return Err(DesignError::Input("the DEM covers no area".into()));
    }
    // Pixels are square on the ground: a degree of longitude is `dlon.to_radians()` radians wide.
    let (wr, hr) = (dlon.to_radians(), dmy);
    let scale = MAX_SIDE / wr.max(hr);
    let (w, h) = (
        (wr * scale).round().max(3.0) as usize,
        (hr * scale).round().max(3.0) as usize,
    );

    // Every pixel centre, as lon/lat, then in the DEM's CRS, then its height.
    let lonlat: Vec<[f64; 2]> = (0..h)
        .flat_map(|r| {
            let lat = lat_of(my_n - (r as f64 + 0.5) / h as f64 * dmy);
            (0..w).map(move |c| [west + (c as f64 + 0.5) / w as f64 * dlon, lat])
        })
        .collect();
    let at_dem = match crs {
        Crs::Wgs84 => lonlat.clone(),
        Crs::Utm(u) => u.forward_all(&lonlat).map_err(DesignError::Input)?,
    };
    let z: Vec<f64> = at_dem
        .iter()
        .map(|p| dem.sample_bilinear(p[0], p[1]))
        .collect();

    // Horn's slope and aspect; light from azimuth 315, altitude 45 (GDAL's defaults).
    let (zenith, light) = (core::f64::consts::FRAC_PI_4, 135f64.to_radians());
    let dx_deg = dlon.to_radians() / w as f64;
    let dy_merc = dmy / h as f64;
    let mut px = vec![0u8; w * h * 2];
    for r in 0..h {
        let cos_lat = libm::cos(lonlat[r * w][1].to_radians());
        let (dx, dy) = (dx_deg * R * cos_lat, dy_merc * R * cos_lat);
        for c in 0..w {
            let g = |dr: i64, dc: i64| {
                let rr = (r as i64 + dr).clamp(0, h as i64 - 1) as usize;
                let cc = (c as i64 + dc).clamp(0, w as i64 - 1) as usize;
                z[rr * w + cc]
            };
            let n = [
                g(-1, -1),
                g(-1, 0),
                g(-1, 1),
                g(0, -1),
                g(0, 1),
                g(1, -1),
                g(1, 0),
                g(1, 1),
            ];
            if z[r * w + c].is_nan() || n.iter().any(|v| v.is_nan()) {
                continue;
            }
            let [a, b, cc, d, f, gg, hh, i] = n;
            let dzdx = ((cc + 2.0 * f + i) - (a + 2.0 * d + gg)) / (8.0 * dx);
            // Rows run south, so "north-positive" flips the sign of the row difference.
            let dzdy = ((a + 2.0 * b + cc) - (gg + 2.0 * hh + i)) / (8.0 * dy);
            let slope = libm::atan(libm::sqrt(dzdx * dzdx + dzdy * dzdy));
            let aspect = libm::atan2(dzdy, -dzdx);
            let shade = libm::cos(zenith) * libm::cos(slope)
                + libm::sin(zenith) * libm::sin(slope) * libm::cos(light - aspect);
            px[(r * w + c) * 2] = (shade.clamp(0.0, 1.0) * 255.0).round() as u8;
            px[(r * w + c) * 2 + 1] = 255;
        }
    }
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w as u32, h as u32);
        enc.set_color(png::ColorType::GrayscaleAlpha);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| DesignError::Input(e.to_string()))?;
        writer
            .write_image_data(&px)
            .map_err(|e| DesignError::Input(e.to_string()))?;
    }
    Ok(Hillshade {
        png,
        width: w as u32,
        height: h as u32,
        corners: [[west, north], [east, north], [east, south], [west, south]],
    })
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    /// A WGS84 DEM of a tilted plane: height rises to the east at 1 m per 10 m.
    fn ramp(rising_east: bool) -> Dem {
        let (w, h) = (40usize, 40usize);
        let step = 0.0001; // about 11 m
        let data = (0..w * h)
            .map(|i| {
                let c = (i % w) as f64;
                let e = c * step * 111_320.0 / 10.0;
                if rising_east { e } else { -e }
            })
            .collect();
        Dem {
            width: w,
            height: h,
            data,
            geotransform: [step, 0.0, -3.0, 0.0, -step, 5.4],
            nodata: Some(-9999.0),
            epsg: Some(4326),
        }
    }

    fn decode(h: &Hillshade) -> (Vec<u8>, usize, usize) {
        let mut r = png::Decoder::new(std::io::Cursor::new(&h.png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        (buf, info.width as usize, info.height as usize)
    }

    fn middle_grey(h: &Hillshade) -> u8 {
        let (px, w, hh) = decode(h);
        px[((hh / 2) * w + w / 2) * 2]
    }

    #[test]
    fn a_slope_facing_the_light_is_brighter_than_one_facing_away() {
        // Lit from the north-west: ground that rises to the east faces west, toward the light.
        let toward = middle_grey(&hillshade(&ramp(true)).unwrap());
        let away = middle_grey(&hillshade(&ramp(false)).unwrap());
        assert!(toward > away + 15, "{toward} vs {away}");
    }

    #[test]
    fn the_picture_is_placed_on_the_dems_own_corners() {
        let h = hillshade(&ramp(true)).unwrap();
        let [tl, tr, br, bl] = h.corners;
        assert_eq!((tl[0], tl[1]), (-3.0, 5.4));
        assert!((tr[0] - (-3.0 + 0.004)).abs() < 1e-12 && (br[1] - (5.4 - 0.004)).abs() < 1e-12);
        assert_eq!(bl, [tl[0], br[1]]);
        let (_, w, hh) = decode(&h);
        assert_eq!((w as u32, hh as u32), (h.width, h.height));
    }

    #[test]
    fn nodata_is_transparent() {
        let mut d = ramp(true);
        for r in 15..25 {
            for c in 15..25 {
                d.data[r * d.width + c] = -9999.0;
            }
        }
        let (px, w, h) = decode(&hillshade(&d).unwrap());
        assert_eq!(px[((h / 2) * w + w / 2) * 2 + 1], 0, "hole is clear");
        assert_eq!(px[((h / 8) * w + w / 8) * 2 + 1], 255, "land is opaque");
    }

    #[test]
    fn a_dem_with_no_crs_is_refused_with_the_reason() {
        let mut d = ramp(true);
        d.epsg = None;
        assert!(hillshade(&d).unwrap_err().to_string().contains("CRS"));
    }
}
