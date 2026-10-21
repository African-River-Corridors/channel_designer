//! Reading a vector input from the bytes of a file.

/// A vector file as GeoJSON text. A GeoPackage is converted; `layer` names one of its layers.
/// `name` is only for the messages.
pub fn vector_text(name: &str, bytes: Vec<u8>, layer: Option<&str>) -> Result<String, String> {
    if owd_format::design::geopackage::is_geopackage(&bytes) {
        owd_format::design::geopackage::to_geojson(&bytes, layer)
            .map_err(|e| format!("{name}: {e}"))
    } else if layer.is_some() {
        Err(format!(
            "{name}: only a GeoPackage has layers, but this is not one"
        ))
    } else {
        String::from_utf8(bytes).map_err(|_| format!("{name}: not GeoJSON text or a GeoPackage"))
    }
}

/// The vector input a role name means.
pub fn role_of(name: &str) -> Result<owd_format::design::Role, String> {
    use owd_format::design::Role;
    match name {
        "left_bank" => Ok(Role::LeftBank),
        "right_bank" => Ok(Role::RightBank),
        "centreline" => Ok(Role::Centreline),
        "river_polygon" => Ok(Role::RiverPolygon),
        other => Err(format!("`{other}` is not an input the app can load")),
    }
}
