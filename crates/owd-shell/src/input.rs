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

/// A bank edit from the JSON the front end sends: `{"kind":"splice","points":[[lon,lat],..]}`,
/// `{"kind":"move","index":n,"to":[lon,lat]}`, `{"kind":"add","after":n,"at":[lon,lat]}` or
/// `{"kind":"delete","index":n}`.
pub fn bank_edit_of(
    edit: &serde_json::Value,
) -> Result<owd_format::design::bank_edit::BankEdit, String> {
    use owd_format::design::bank_edit::BankEdit;
    let pt = |v: &serde_json::Value| -> Result<[f64; 2], String> {
        match (
            v.get(0).and_then(|x| x.as_f64()),
            v.get(1).and_then(|x| x.as_f64()),
        ) {
            (Some(x), Some(y)) => Ok([x, y]),
            _ => Err("a point must be [longitude, latitude]".into()),
        }
    };
    let idx = |k: &str| -> Result<usize, String> {
        edit[k]
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| format!("`{k}` must be a vertex number"))
    };
    match edit["kind"].as_str() {
        Some("splice") => Ok(BankEdit::Splice(
            edit["points"]
                .as_array()
                .ok_or("a splice needs `points`")?
                .iter()
                .map(pt)
                .collect::<Result<_, _>>()?,
        )),
        Some("move") => Ok(BankEdit::Move {
            index: idx("index")?,
            to: pt(&edit["to"])?,
        }),
        Some("add") => Ok(BankEdit::Add {
            after: idx("after")?,
            at: pt(&edit["at"])?,
        }),
        Some("delete") => Ok(BankEdit::Delete {
            index: idx("index")?,
        }),
        other => Err(format!("`{}` is not a bank edit", other.unwrap_or("?"))),
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use owd_format::design::bank_edit::BankEdit;
    use serde_json::json;

    #[test]
    fn the_four_edits_are_read_and_a_bad_one_is_refused() {
        assert_eq!(
            bank_edit_of(&json!({"kind":"delete","index":3})),
            Ok(BankEdit::Delete { index: 3 })
        );
        assert_eq!(
            bank_edit_of(&json!({"kind":"move","index":1,"to":[1.5,2.5]})),
            Ok(BankEdit::Move {
                index: 1,
                to: [1.5, 2.5]
            })
        );
        assert_eq!(
            bank_edit_of(&json!({"kind":"add","after":0,"at":[1,2]})),
            Ok(BankEdit::Add {
                after: 0,
                at: [1.0, 2.0]
            })
        );
        assert!(
            matches!(bank_edit_of(&json!({"kind":"splice","points":[[1,2],[3,4]]})), Ok(BankEdit::Splice(p)) if p.len() == 2)
        );
        assert!(bank_edit_of(&json!({"kind":"move","index":-1,"to":[1,2]})).is_err());
        assert!(bank_edit_of(&json!({"kind":"explode"})).is_err());
        assert!(bank_edit_of(&json!({"kind":"splice","points":[[1]]})).is_err());
    }
}
