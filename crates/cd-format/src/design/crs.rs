//! Coordinate systems for v1 (ADR 0003): WGS84 (EPSG:4326) and WGS84 / UTM (EPSG:326xx north,
//! EPSG:327xx south). Anything else is refused with a message that says what to convert to.
//!
//! Reprojection is `proj4rs` (pure Rust, `+proj=utm`); it is tested against PROJ's own output to
//! 1 mm (`tests/projection.rs`).

use proj4rs::proj::Proj;

/// A WGS84 / UTM zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utm {
    /// 1 to 60.
    pub zone: u8,
    pub south: bool,
}

/// A CRS the tool can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crs {
    Wgs84,
    Utm(Utm),
}

impl Utm {
    /// The zone that holds a WGS84 point (longitude and latitude in degrees).
    pub fn for_point(lon: f64, lat: f64) -> Utm {
        let lon = ((lon + 180.0).rem_euclid(360.0)) - 180.0;
        let zone = (((lon + 180.0) / 6.0).floor() as i64 + 1).clamp(1, 60) as u8;
        Utm {
            zone,
            south: lat < 0.0,
        }
    }

    pub fn epsg(self) -> u32 {
        (if self.south { 32700 } else { 32600 }) + u32::from(self.zone)
    }

    pub fn name(self) -> String {
        format!(
            "WGS 84 / UTM zone {}{}",
            self.zone,
            if self.south { "S" } else { "N" }
        )
    }

    fn proj_string(self) -> String {
        format!(
            "+proj=utm +zone={}{} +datum=WGS84 +units=m +no_defs",
            self.zone,
            if self.south { " +south" } else { "" }
        )
    }

    /// WGS84 longitude and latitude (degrees) to easting and northing (m).
    pub fn forward(self, lon: f64, lat: f64) -> Result<[f64; 2], String> {
        let geo = Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
            .map_err(|e| e.to_string())?;
        let utm = Proj::from_proj_string(&self.proj_string()).map_err(|e| e.to_string())?;
        let mut p = (lon.to_radians(), lat.to_radians(), 0.0);
        proj4rs::transform::transform(&geo, &utm, &mut p).map_err(|e| e.to_string())?;
        Ok([p.0, p.1])
    }

    /// Easting and northing (m) to WGS84 longitude and latitude (degrees).
    pub fn inverse(self, x: f64, y: f64) -> Result<[f64; 2], String> {
        let geo = Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
            .map_err(|e| e.to_string())?;
        let utm = Proj::from_proj_string(&self.proj_string()).map_err(|e| e.to_string())?;
        let mut p = (x, y, 0.0);
        proj4rs::transform::transform(&utm, &geo, &mut p).map_err(|e| e.to_string())?;
        Ok([p.0.to_degrees(), p.1.to_degrees()])
    }
}

impl Crs {
    /// The CRS an EPSG code names, if the tool can use it.
    pub fn from_epsg(code: u32) -> Result<Crs, String> {
        match code {
            4326 => Ok(Crs::Wgs84),
            32601..=32660 => Ok(Crs::Utm(Utm {
                zone: (code - 32600) as u8,
                south: false,
            })),
            32701..=32760 => Ok(Crs::Utm(Utm {
                zone: (code - 32700) as u8,
                south: true,
            })),
            other => Err(format!(
                "EPSG:{other} is not supported. Convert the data to WGS84 (EPSG:4326) or to \
                 WGS84 / UTM (EPSG:326xx for the northern hemisphere, EPSG:327xx for the southern)."
            )),
        }
    }

    /// Parse `EPSG:32630`, `urn:ogc:def:crs:EPSG::32630`, `urn:ogc:def:crs:EPSG:6.6:32630` or
    /// the OGC84 alias for WGS84.
    pub fn parse(name: &str) -> Result<Crs, String> {
        let upper = name.trim().to_ascii_uppercase();
        if upper.ends_with("CRS84") || upper.ends_with("OGC:1.3:CRS84") {
            return Ok(Crs::Wgs84);
        }
        let code = upper
            .rsplit([':', '/'])
            .next()
            .and_then(|s| s.parse::<u32>().ok());
        match (upper.contains("EPSG"), code) {
            (true, Some(c)) => Crs::from_epsg(c),
            _ => Err(format!(
                "the CRS `{name}` is not supported. Convert the data to WGS84 (EPSG:4326) or to \
                 WGS84 / UTM (EPSG:326xx for the northern hemisphere, EPSG:327xx for the southern)."
            )),
        }
    }

    pub fn epsg(self) -> u32 {
        match self {
            Crs::Wgs84 => 4326,
            Crs::Utm(u) => u.epsg(),
        }
    }
}

impl Utm {
    /// [`Utm::inverse`] for many points, building the projection once.
    pub fn inverse_all(self, pts: &[[f64; 2]]) -> Result<Vec<[f64; 2]>, String> {
        let geo = Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
            .map_err(|e| e.to_string())?;
        let utm = Proj::from_proj_string(&self.proj_string()).map_err(|e| e.to_string())?;
        pts.iter()
            .map(|&[x, y]| {
                let mut p = (x, y, 0.0);
                proj4rs::transform::transform(&utm, &geo, &mut p).map_err(|e| e.to_string())?;
                Ok([p.0.to_degrees(), p.1.to_degrees()])
            })
            .collect()
    }
}

impl Utm {
    /// [`Utm::forward`] for many points, building the projection once.
    pub fn forward_all(self, pts: &[[f64; 2]]) -> Result<Vec<[f64; 2]>, String> {
        let geo = Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
            .map_err(|e| e.to_string())?;
        let utm = Proj::from_proj_string(&self.proj_string()).map_err(|e| e.to_string())?;
        pts.iter()
            .map(|&[lon, lat]| {
                let mut p = (lon.to_radians(), lat.to_radians(), 0.0);
                proj4rs::transform::transform(&geo, &utm, &mut p).map_err(|e| e.to_string())?;
                Ok([p.0, p.1])
            })
            .collect()
    }
}
