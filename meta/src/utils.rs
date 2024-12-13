use cfg_expr::{targets::get_builtin_target_by_triple, Expression, Predicate};
use toml::{Table, Value};

use crate::error::Error;

/// Base merge function to use with `read_metadata`.
/// It will join `serde_json` values based on some assignment rules.
pub fn merge_default(rhs: &mut Table, lhs: Table, overwrite: bool) -> Result<(), Error> {
    for (key, lhs) in lhs {
        // 1. None = * will always return the new value.
        let Some(rhs) = rhs.get_mut(&key) else {
            rhs.insert(key, lhs);
            continue;
        };

        // 2. If they are the same, we can stop early
        if *rhs == lhs {
            continue;
        }

        // 3. Assignment from two different types is incompatible.
        if std::mem::discriminant(rhs) != std::mem::discriminant(&lhs) {
            return Err(Error::IncompatibleMerge);
        }

        match (rhs, lhs) {
            // 4. Arrays return a combined deduplicated list.
            (Value::Array(rhs), Value::Array(lhs)) => {
                for value in lhs {
                    if !rhs.contains(&value) {
                        rhs.push(value);
                    }
                }
            }
            // 5. Tables combine keys from both following the previous rules.
            (Value::Table(rhs), Value::Table(lhs)) => {
                merge_default(rhs, lhs, overwrite)?;
            }
            // 6. For simple types (Booleans, Numbers and Strings):
            //   6.1. If overwrite is true, the new value will be returned.
            //   6.2. Otherwise, if the value is not the same there will be an error.
            (r, l) => {
                if !overwrite {
                    return Err(Error::IncompatibleMerge);
                }
                *r = l;
            }
        }
    }
    Ok(())
}

/// ```toml
/// [package.metadata.'cfg(target = "unix")']
/// value = ...
/// ```
pub fn reduce(table: Table) -> Result<Table, Error> {
    let mut res = Table::new();
    let mut conditionals = Table::new();

    for (key, value) in table {
        // Conditional expressions
        if let Some(cfg) = key.strip_prefix("cfg(") {
            let pred = cfg
                .strip_suffix(")")
                .ok_or(Error::UnsupportedCfg(key.clone()))?;
            if !check_cfg(pred)? {
                continue;
            };
            let Value::Table(inner) = value else {
                return Err(Error::CfgNotObject(key));
            };
            for (inner_key, value) in inner {
                let Value::Table(value) = value else {
                    return Err(Error::CfgNotObject(key));
                };
                let prev = conditionals
                    .entry(inner_key)
                    .or_insert(Value::Table(Table::new()));

                merge_default(prev.as_table_mut().unwrap(), value, false)?;
            }
            continue;
        }

        // General case
        res.insert(
            key,
            match value {
                Value::Table(t) => Value::Table(reduce(t)?),
                v => v,
            },
        );
    }

    // Conditionals can overwrite the default case
    merge_default(&mut res, conditionals, true)?;
    Ok(res)
}

fn check_cfg(pred: &str) -> Result<bool, Error> {
    let target = get_builtin_target_by_triple(env!("TARGET"))
        .expect("The target set by the build script should be valid");
    let expr = Expression::parse(pred).map_err(Error::InvalidCfg)?;
    let res = expr.eval(|pred| match pred {
        Predicate::Target(p) => Some(p.matches(target)),
        _ => None,
    });
    res.ok_or(Error::UnsupportedCfg(pred.into()))
}
