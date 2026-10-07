//! Built-in functions. Speran has no methods: everything is a plain function
//! call, which gives LLMs exactly one way to do each thing.

use crate::error::{ErrorKind, SperanError};
use crate::value::{Map, Value};
use std::cmp::Ordering;

type R = Result<Value, SperanError>;

/// (name, min positional args, max positional args, signature)
pub const BUILTINS: &[(&str, usize, usize, &str)] = &[
    ("len", 1, 1, "len(x)"),
    ("str", 1, 1, "str(x)"),
    ("num", 1, 1, "num(x)"),
    ("int", 1, 1, "int(x)"),
    ("bool", 1, 1, "bool(x)"),
    ("type", 1, 1, "type(x)"),
    ("range", 1, 3, "range(end) / range(start, end, step?)"),
    ("keys", 1, 1, "keys(map)"),
    ("values", 1, 1, "values(map)"),
    ("items", 1, 1, "items(map)"),
    ("get", 2, 3, "get(map_or_list, key, default?)"),
    ("sort", 1, 1, "sort(list, by: \"field\"?, reverse: bool?)"),
    ("reverse", 1, 1, "reverse(list_or_string)"),
    ("sum", 1, 1, "sum(list)"),
    ("min", 1, usize::MAX, "min(list) / min(a, b, ...)"),
    ("max", 1, usize::MAX, "max(list) / max(a, b, ...)"),
    ("join", 1, 2, "join(list, sep?)"),
    ("split", 1, 2, "split(string, sep?)"),
    ("upper", 1, 1, "upper(string)"),
    ("lower", 1, 1, "lower(string)"),
    ("trim", 1, 1, "trim(string)"),
    ("replace", 3, 3, "replace(string, old, new)"),
    ("starts_with", 2, 2, "starts_with(string, prefix)"),
    ("ends_with", 2, 2, "ends_with(string, suffix)"),
    ("round", 1, 2, "round(x, digits?)"),
    ("abs", 1, 1, "abs(x)"),
    ("pow", 2, 2, "pow(base, exponent)"),
    ("enumerate", 1, 1, "enumerate(list)"),
    ("zip", 2, 2, "zip(a, b)"),
    ("unique", 1, 1, "unique(list)"),
    ("flatten", 1, 1, "flatten(list)"),
    ("to_json", 1, 1, "to_json(x)"),
    ("parse_json", 1, 1, "parse_json(string)"),
];

fn lookup(name: &str) -> Option<&'static (&'static str, usize, usize, &'static str)> {
    BUILTINS.iter().find(|b| b.0 == name)
}

pub fn is_builtin(name: &str) -> bool {
    lookup(name).is_some()
}

pub fn names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|b| b.0)
}

pub fn signatures() -> String {
    BUILTINS.iter().map(|b| b.3).collect::<Vec<_>>().join(", ")
}

fn allowed_kwargs(name: &str) -> &'static [&'static str] {
    match name {
        "sort" => &["by", "reverse"],
        _ => &[],
    }
}

pub fn check_arity<'a>(name: &str, npos: usize, kw: impl Iterator<Item = &'a str>) -> Result<(), SperanError> {
    let Some(&(_, min, max, sig)) = lookup(name) else {
        return Err(SperanError::new(ErrorKind::UnknownName, format!("Unknown function '{}'", name)));
    };
    if npos < min || npos > max {
        let expected = if min == max {
            format!("{}", min)
        } else if max == usize::MAX {
            format!("at least {}", min)
        } else {
            format!("{} to {}", min, max)
        };
        return Err(SperanError::new(
            ErrorKind::ArgumentError,
            format!("{}() takes {} positional argument(s) but {} were given", name, expected, npos),
        )
        .with_hint(format!("Usage: {}", sig)));
    }
    for k in kw {
        if !allowed_kwargs(name).contains(&k) {
            return Err(SperanError::new(
                ErrorKind::ArgumentError,
                format!("{}() has no parameter named '{}'", name, k),
            )
            .with_hint(format!("Usage: {}", sig)));
        }
    }
    Ok(())
}

/// Hint for when an LLM writes `x.upper()`-style method calls.
pub fn method_hint(method: &str, obj: &str) -> String {
    let fix = match method {
        "upper" | "toUpperCase" => format!("upper({obj})"),
        "lower" | "toLowerCase" => format!("lower({obj})"),
        "strip" | "trim" => format!("trim({obj})"),
        "split" => format!("split({obj}, sep)"),
        "join" => format!("join(items, {obj})"),
        "keys" | "values" | "items" => format!("{method}({obj})"),
        "get" => format!("get({obj}, key, default)"),
        "startswith" | "startsWith" => format!("starts_with({obj}, prefix)"),
        "endswith" | "endsWith" => format!("ends_with({obj}, suffix)"),
        "replace" => format!("replace({obj}, old, new)"),
        "sort" | "sorted" => format!("sort({obj})"),
        "reverse" => format!("reverse({obj})"),
        "length" | "len" | "size" | "count" => format!("len({obj})"),
        "append" | "push" | "add" | "extend" | "insert" => {
            return format!("Values are immutable. Use: {obj} = {obj} + [item]")
        }
        "update" | "set" => return format!("Values are immutable. Use: {obj} = {obj} + {{key: value}}"),
        "includes" | "contains" | "has" => return format!("Use the 'in' operator: x in {obj}"),
        "map" | "filter" | "forEach" => {
            return format!("Use a comprehension: [f(x) for x in {obj} if cond]")
        }
        "format" => return "Use string interpolation: \"Hello {name}\"".to_string(),
        _ => {
            return format!("Speran has no methods; use built-in functions. Built-ins: {}", signatures())
        }
    };
    format!("Speran has no methods. Use {fix} instead.")
}

fn type_err(msg: impl Into<String>) -> SperanError {
    SperanError::new(ErrorKind::TypeError, msg)
}

fn value_err(msg: impl Into<String>) -> SperanError {
    SperanError::new(ErrorKind::ValueError, msg)
}

fn arg_num(f: &str, v: &Value) -> Result<f64, SperanError> {
    match v {
        Value::Num(n) => Ok(*n),
        o => Err(type_err(format!("{}() expects a number, got {}", f, o.type_name()))),
    }
}

fn arg_int(f: &str, v: &Value) -> Result<i64, SperanError> {
    let n = arg_num(f, v)?;
    if n.fract() != 0.0 {
        return Err(type_err(format!("{}() expects a whole number, got {}", f, n)));
    }
    Ok(n as i64)
}

fn arg_str<'v>(f: &str, v: &'v Value) -> Result<&'v str, SperanError> {
    match v {
        Value::Str(s) => Ok(s),
        o => Err(type_err(format!("{}() expects a string, got {}", f, o.type_name()))
            .with_hint("Convert with str(x).")),
    }
}

fn arg_list<'v>(f: &str, v: &'v Value) -> Result<&'v Vec<Value>, SperanError> {
    match v {
        Value::List(l) => Ok(l),
        o => Err(type_err(format!("{}() expects a list, got {}", f, o.type_name()))),
    }
}

fn arg_map<'v>(f: &str, v: &'v Value) -> Result<&'v Map, SperanError> {
    match v {
        Value::Map(m) => Ok(m),
        o => Err(type_err(format!("{}() expects a map, got {}", f, o.type_name()))),
    }
}

pub fn compare(f: &str, a: &Value, b: &Value) -> Result<Ordering, SperanError> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => Ok(x.partial_cmp(y).unwrap_or(Ordering::Equal)),
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        _ => Err(type_err(format!(
            "{}() cannot compare {} with {}",
            f,
            a.type_name(),
            b.type_name()
        ))
        .with_hint("Only numbers with numbers, or strings with strings, can be ordered.")),
    }
}

fn pair(a: Value, b: Value) -> Value {
    Value::list(vec![a, b])
}

pub fn call(name: &str, args: &[Value], kw: &Map) -> R {
    check_arity(name, args.len(), kw.keys().map(|k| k.as_str()))?;
    let a0 = &args[0];
    Ok(match name {
        "len" => match a0 {
            Value::Str(s) => Value::Num(s.chars().count() as f64),
            Value::List(l) => Value::Num(l.len() as f64),
            Value::Map(m) => Value::Num(m.len() as f64),
            o => return Err(type_err(format!("len() expects a string, list or map, got {}", o.type_name()))),
        },
        "str" => Value::str(a0.display()),
        "num" | "int" => {
            let n = match a0 {
                Value::Num(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Str(s) => s.trim().parse::<f64>().map_err(|_| {
                    value_err(format!("Cannot convert \"{}\" to a number", s))
                })?,
                o => return Err(type_err(format!("{}() cannot convert a {} to a number", name, o.type_name()))),
            };
            Value::Num(if name == "int" { n.trunc() } else { n })
        }
        "bool" => Value::Bool(a0.truthy()),
        "type" => Value::str(a0.type_name()),
        "range" => {
            let (start, end, step) = match args.len() {
                1 => (0, arg_int(name, a0)?, 1),
                2 => (arg_int(name, a0)?, arg_int(name, &args[1])?, 1),
                _ => (arg_int(name, a0)?, arg_int(name, &args[1])?, arg_int(name, &args[2])?),
            };
            if step == 0 {
                return Err(value_err("range() step cannot be 0"));
            }
            let count = if step > 0 {
                ((end - start).max(0) + step - 1) / step
            } else {
                ((start - end).max(0) + (-step) - 1) / (-step)
            };
            if count > 1_000_000 {
                return Err(value_err(format!("range() would create {} items (max 1000000)", count)));
            }
            let mut v = Vec::with_capacity(count as usize);
            let mut i = start;
            while (step > 0 && i < end) || (step < 0 && i > end) {
                v.push(Value::Num(i as f64));
                i += step;
            }
            Value::list(v)
        }
        "keys" => Value::list(arg_map(name, a0)?.keys().map(|k| Value::str(k.clone())).collect()),
        "values" => Value::list(arg_map(name, a0)?.values().cloned().collect()),
        "items" => Value::list(
            arg_map(name, a0)?
                .iter()
                .map(|(k, v)| pair(Value::str(k.clone()), v.clone()))
                .collect(),
        ),
        "get" => {
            let default = args.get(2).cloned().unwrap_or(Value::Null);
            match (a0, &args[1]) {
                (Value::Map(m), Value::Str(k)) => m.get(&**k).cloned().unwrap_or(default),
                (Value::List(l), Value::Num(i)) => {
                    let len = l.len() as i64;
                    let i = *i as i64;
                    let i = if i < 0 { i + len } else { i };
                    if (0..len).contains(&i) {
                        l[i as usize].clone()
                    } else {
                        default
                    }
                }
                (Value::Null, _) => default,
                (o, k) => {
                    return Err(type_err(format!(
                        "get() expects a map with a string key or a list with a number index, got {} and {}",
                        o.type_name(),
                        k.type_name()
                    )))
                }
            }
        }
        "sort" => sort(a0, kw)?,
        "reverse" => match a0 {
            Value::List(l) => Value::list(l.iter().rev().cloned().collect()),
            Value::Str(s) => Value::str(s.chars().rev().collect::<String>()),
            o => return Err(type_err(format!("reverse() expects a list or string, got {}", o.type_name()))),
        },
        "sum" => {
            let mut total = 0.0;
            for v in arg_list(name, a0)?.iter() {
                match v {
                    Value::Num(n) => total += n,
                    o => return Err(type_err(format!("sum() expects a list of numbers, found a {}", o.type_name()))),
                }
            }
            Value::Num(total)
        }
        "min" | "max" => {
            let items: Vec<Value> = match (args.len(), a0) {
                (1, Value::List(l)) => (**l).clone(),
                _ => args.to_vec(),
            };
            let Some(first) = items.first() else {
                return Err(value_err(format!("{}() of an empty list", name)));
            };
            let mut best = first.clone();
            for v in &items[1..] {
                let ord = compare(name, v, &best)?;
                if (name == "min" && ord == Ordering::Less) || (name == "max" && ord == Ordering::Greater) {
                    best = v.clone();
                }
            }
            best
        }
        "join" => {
            let l = arg_list(name, a0)?;
            let sep = if args.len() > 1 { arg_str(name, &args[1])? } else { "" };
            Value::str(l.iter().map(Value::display).collect::<Vec<_>>().join(sep))
        }
        "split" => {
            let s = arg_str(name, a0)?;
            let parts: Vec<Value> = if args.len() > 1 {
                let sep = arg_str(name, &args[1])?;
                if sep.is_empty() {
                    return Err(value_err("split() separator cannot be empty"));
                }
                s.split(sep).map(Value::str).collect()
            } else {
                s.split_whitespace().map(Value::str).collect()
            };
            Value::list(parts)
        }
        "upper" => Value::str(arg_str(name, a0)?.to_uppercase()),
        "lower" => Value::str(arg_str(name, a0)?.to_lowercase()),
        "trim" => Value::str(arg_str(name, a0)?.trim()),
        "replace" => {
            let s = arg_str(name, a0)?;
            let old = arg_str(name, &args[1])?;
            let new = arg_str(name, &args[2])?;
            if old.is_empty() {
                return Err(value_err("replace() 'old' cannot be empty"));
            }
            Value::str(s.replace(old, new))
        }
        "starts_with" => Value::Bool(arg_str(name, a0)?.starts_with(arg_str(name, &args[1])?)),
        "ends_with" => Value::Bool(arg_str(name, a0)?.ends_with(arg_str(name, &args[1])?)),
        "round" => {
            let n = arg_num(name, a0)?;
            let digits = if args.len() > 1 { arg_int(name, &args[1])? } else { 0 };
            let f = 10f64.powi(digits.clamp(-15, 15) as i32);
            Value::Num((n * f).round() / f)
        }
        "abs" => Value::Num(arg_num(name, a0)?.abs()),
        "pow" => Value::Num(arg_num(name, a0)?.powf(arg_num(name, &args[1])?)),
        "enumerate" => Value::list(
            arg_list(name, a0)?
                .iter()
                .enumerate()
                .map(|(i, v)| pair(Value::Num(i as f64), v.clone()))
                .collect(),
        ),
        "zip" => {
            let a = arg_list(name, a0)?;
            let b = arg_list(name, &args[1])?;
            Value::list(a.iter().zip(b.iter()).map(|(x, y)| pair(x.clone(), y.clone())).collect())
        }
        "unique" => {
            let mut out: Vec<Value> = Vec::new();
            for v in arg_list(name, a0)?.iter() {
                if !out.contains(v) {
                    out.push(v.clone());
                }
            }
            Value::list(out)
        }
        "flatten" => {
            let mut out = Vec::new();
            for v in arg_list(name, a0)?.iter() {
                match v {
                    Value::List(inner) => out.extend(inner.iter().cloned()),
                    o => out.push(o.clone()),
                }
            }
            Value::list(out)
        }
        "to_json" => Value::str(a0.repr()),
        "parse_json" => {
            let s = arg_str(name, a0)?;
            let j: serde_json::Value =
                serde_json::from_str(s).map_err(|e| value_err(format!("Invalid JSON: {}", e)))?;
            Value::from_json(&j)
        }
        _ => return Err(SperanError::new(ErrorKind::UnknownName, format!("Unknown function '{}'", name))),
    })
}

fn sort(list: &Value, kw: &Map) -> R {
    let l = arg_list("sort", list)?;
    let by = match kw.get("by") {
        None | Some(Value::Null) => None,
        Some(Value::Str(s)) => Some(s.to_string()),
        Some(_) => return Err(type_err("sort() 'by' must be a field name string, e.g. by: \"score\"")),
    };
    let reverse = kw.get("reverse").is_some_and(Value::truthy);
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(l.len());
    for item in l.iter() {
        let key = match (&by, item) {
            (None, _) => item.clone(),
            (Some(f), Value::Map(m)) => m.get(f.as_str()).cloned().ok_or_else(|| {
                SperanError::new(ErrorKind::KeyError, format!("sort(by: \"{}\"): an item has no key '{}'", f, f))
            })?,
            (Some(_), o) => {
                return Err(type_err(format!("sort(by: ...) expects a list of maps, found a {}", o.type_name())))
            }
        };
        keyed.push((key, item.clone()));
    }
    if let Some((first, _)) = keyed.first() {
        for (k, _) in &keyed {
            compare("sort", first, k)?;
        }
    }
    keyed.sort_by(|a, b| compare("sort", &a.0, &b.0).unwrap_or(Ordering::Equal));
    if reverse {
        keyed.reverse();
    }
    Ok(Value::list(keyed.into_iter().map(|(_, v)| v).collect()))
}
