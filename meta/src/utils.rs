use cfg_expr::{targets::get_builtin_target_by_triple, Expression, Predicate};
use serde_json::{Map, Value};

use crate::error::{CfgError, Error};

pub fn merge_base(
    rhs: Value,
    lhs: &Value,
    overwrite: bool,
    merge: &impl Fn(Value, Value, bool) -> Result<Value, Error>,
) -> Result<Value, Error> {
    // 1. If they are the same, we can stop early
    if rhs == *lhs {
        return Ok(rhs);
    }

    // 2.1. (* = Null) will always return the old value.
    if let Value::Null = lhs {
        return Ok(rhs);
    }
    // 2.2. (Null = *) will always return the new value.
    let lhs = reduce_default(lhs.clone())?;
    if let Value::Null = rhs {
        return Ok(lhs);
    }

    // 3. Assignment from two different types (excluding `Null`) is incompatible.
    if std::mem::discriminant(&rhs) != std::mem::discriminant(&lhs) {
        return Err(Error::IncompatibleMerge);
    }

    merge(rhs, lhs, overwrite)
}

/// Base merge function to use with `MetadataList::get`.
/// It will join `serde_json` values based on some assignment rules.
pub fn merge_default(rhs: Value, lhs: Value, overwrite: bool) -> Result<Value, Error> {
    match (rhs, lhs) {
        // 4. Arrays return a combined deduplicated list.
        (Value::Array(mut r), Value::Array(l)) => {
            for v in l {
                if !r.contains(&v) {
                    r.push(v.clone());
                }
            }
            Ok(Value::Array(r))
        }
        // 5. Objects combine keys from both following the previous rules.
        (Value::Object(mut r), Value::Object(l)) => {
            for (k, v) in l {
                let merged = merge_base(
                    r.remove(&k).unwrap_or(Value::Null),
                    &v,
                    overwrite,
                    &merge_default,
                )?;
                r.insert(k, merged);
            }
            Ok(Value::Object(r))
        }
        // 6. For simple types (Booleans, Numbers and Strings):
        //   6.1. If overwrite is true, the new value will be returned.
        //   6.2. Otherwise, if the value is not the same there will be an error.
        (_, l) => {
            if overwrite {
                Ok(l)
            } else {
                Err(Error::IncompatibleMerge)
            }
        }
    }
}

/// ```toml
/// [package.metadata.'cfg(target = "unix")']
/// value = ...
/// ```
pub fn reduce_default(value: Value) -> Result<Value, Error> {
    let Value::Object(map) = value else {
        return Ok(value);
    };

    let mut res = Map::new();
    let mut conditionals = Map::new();

    for (k, v) in map.into_iter() {
        // Conditional expressions
        if let Some(cfg) = k.strip_prefix("cfg(") {
            let pred = cfg
                .strip_suffix(")")
                .ok_or(CfgError::Unsupported(k.clone()))?;
            if !check_cfg(pred)? {
                continue;
            };
            let Value::Object(map) = reduce_default(v)? else {
                return Err(CfgError::NotObject.into());
            };
            for (k, v) in map {
                if !v.is_object() {
                    return Err(CfgError::NotObject.into());
                }
                let prev = conditionals.get(&k).cloned().unwrap_or(Value::Null);
                conditionals.insert(k, merge_base(prev, &v, false, &merge_default)?);
            }
            continue;
        }

        // General case
        res.insert(k, reduce_default(v)?);
    }

    // Conditionals can overwrite the default case
    let res = merge_base(
        Value::Object(res),
        &Value::Object(conditionals),
        true,
        &merge_default,
    )?;

    Ok(res)
}

fn check_cfg(pred: &str) -> Result<bool, CfgError> {
    let target = get_builtin_target_by_triple(env!("TARGET"))
        .expect("The target set by the build script should be valid");
    let expr = Expression::parse(pred).map_err(CfgError::Invalid)?;
    let res = expr.eval(|pred| match pred {
        Predicate::Target(tp) => Some(tp.matches(target)),
        _ => None,
    });
    res.ok_or(CfgError::Unsupported(pred.into()))
}
