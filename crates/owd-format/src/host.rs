//! Python callables, as a call back into the host (ticket 35).
//!
//! The parity cases describe every callable as a spec (`{"kind": "plane", ...}`). A Python user
//! passes a real function: a terrain sampler, a grade, a cost. The host (the `owd-py` extension)
//! installs a [`Host`] for the length of one `eval` call, and a spec `{"kind": "callback", "id": n}`
//! is then read as a call to that host.
//!
//! A callback cannot raise through Rust. The host keeps the first error it saw, answers NaN (or an
//! empty list) from then on, and raises the error when `eval` returns.

use super::EvalError;
use serde_json::Value;
use std::cell::RefCell;
use std::sync::Arc;

/// One argument or result of a callback.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    F(f64),
    A(Vec<f64>),
    S(String),
    J(Value),
}

/// What the host must answer.
pub trait Host: Send + Sync {
    fn call(&self, id: u64, args: &[Val]) -> Result<Val, String>;
}

thread_local! {
    static HOST: RefCell<Option<Arc<dyn Host>>> = const { RefCell::new(None) };
}

/// Run `f` with `host` installed.
pub fn with_host<R>(host: Arc<dyn Host>, f: impl FnOnce() -> R) -> R {
    let previous = HOST.with(|h| h.replace(Some(host)));
    let out = f();
    HOST.with(|h| *h.borrow_mut() = previous);
    out
}

fn current() -> Result<Arc<dyn Host>, EvalError> {
    HOST.with(|h| h.borrow().clone())
        .ok_or_else(|| EvalError::BadInput("a callback spec needs a host that supplies it".into()))
}

/// The id of a callback spec, or `None` when `spec` is not one.
pub fn callback_id(spec: &Value) -> Option<u64> {
    if spec.get("kind").and_then(Value::as_str) == Some("callback") {
        spec.get("id").and_then(Value::as_u64)
    } else {
        None
    }
}

/// `f(x) -> y`.
pub type ScalarFn = Box<dyn Fn(f64) -> f64>;
/// `f(a, b, c) -> y`.
pub type TripleFn = Box<dyn Fn(f64, f64, f64) -> f64>;
/// `f(xs, ys) -> zs`.
pub type ArraysFn = Box<dyn Fn(&[f64], &[f64]) -> Vec<f64>>;
/// `f(x) -> text`.
pub type TextFn = Box<dyn Fn(f64) -> String>;
/// A call with JSON in and out.
pub type JsonFn = Box<dyn Fn(Value) -> Option<Value> + Send + Sync>;

/// `f(x) -> y`.
pub fn scalar_fn(id: u64) -> Result<ScalarFn, EvalError> {
    let h = current()?;
    Ok(Box::new(move |x| match h.call(id, &[Val::F(x)]) {
        Ok(Val::F(y)) => y,
        _ => f64::NAN,
    }))
}

/// `f(a, b, c) -> y`, which may refuse.
pub fn triple_fn(id: u64) -> Result<TripleFn, EvalError> {
    let h = current()?;
    Ok(Box::new(move |a, b, c| {
        match h.call(id, &[Val::F(a), Val::F(b), Val::F(c)]) {
            Ok(Val::F(y)) => y,
            _ => f64::NAN,
        }
    }))
}

/// `f(xs, ys) -> zs`.
pub fn arrays_fn(id: u64) -> Result<ArraysFn, EvalError> {
    let h = current()?;
    Ok(Box::new(move |xs, ys| {
        match h.call(id, &[Val::A(xs.to_vec()), Val::A(ys.to_vec())]) {
            Ok(Val::A(z)) => z,
            _ => vec![f64::NAN; xs.len()],
        }
    }))
}

/// `f(s) -> text`.
pub fn text_fn(id: u64) -> Result<TextFn, EvalError> {
    let h = current()?;
    Ok(Box::new(move |s| match h.call(id, &[Val::F(s)]) {
        Ok(Val::S(t)) => t,
        _ => String::new(),
    }))
}

/// A call with JSON in and JSON out, for the rare callable that takes a mapping (a width method).
pub fn json_fn(id: u64) -> Result<JsonFn, EvalError> {
    let h = current()?;
    Ok(Box::new(move |v| match h.call(id, &[Val::J(v)]) {
        Ok(Val::J(r)) => Some(r),
        _ => None,
    }))
}
