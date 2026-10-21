//! A DEM as a terrain sampler in the project CRS. The DEM is never warped: each query point is
//! converted from the project CRS to the DEM's CRS and sampled there, bilinear, as the core does.

use super::crs::Crs;
use super::session::DesignError;
use crate::geotiff::Dem;

/// The CRS of a DEM as the document records it: `EPSG:nnnn`, or `unknown`.
pub fn dem_crs_name(dem: &Dem) -> String {
    dem.epsg
        .map_or("unknown".to_string(), |e| format!("EPSG:{e}"))
}

/// A sampler over `dem` for points in the project CRS (`project_epsg`, a WGS84 / UTM code).
/// A DEM in WGS84 is sampled where it is; a DEM in the project's own zone is sampled directly;
/// any other CRS is refused with a message that says what to convert to.
pub fn dem_sampler(
    dem: &Dem,
    project_epsg: Option<u32>,
) -> Result<impl Fn(&[f64], &[f64]) -> Vec<f64> + '_, DesignError> {
    let project = project_epsg.and_then(|c| Crs::from_epsg(c).ok());
    let utm = match (dem.epsg, project) {
        (Some(4326), Some(Crs::Utm(u))) => Some(u),
        (e, Some(p)) if e == Some(p.epsg()) => None,
        (e, Some(p)) => {
            return Err(DesignError::Input(format!(
                "the DEM's CRS ({}) is not WGS84 and not the channel design's EPSG:{}. \
                 Convert the DEM to one of them.",
                e.map_or("unknown".to_string(), |e| format!("EPSG:{e}")),
                p.epsg()
            )));
        }
        (_, None) => {
            return Err(DesignError::Input(
                "the channel design has no project CRS yet: import the river first".into(),
            ));
        }
    };
    Ok(move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let p = match utm {
                    Some(u) => u.inverse(x, y).unwrap_or([f64::NAN; 2]),
                    None => [x, y],
                };
                dem.sample_bilinear(p[0], p[1])
            })
            .collect()
    })
}
