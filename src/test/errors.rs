use crate::errors::{ParrotError, verify};

#[test]
fn test_verify_bools() {
    let x = true;
    let x = verify(x, ParrotError::Other("not true"));
    assert_eq!(x, Ok(true));

    let x = false;
    let x = verify(x, ParrotError::Other("not true"));
    assert_eq!(x, Err(ParrotError::Other("not true")));
}

#[test]
fn test_verify_options() {
    let x = Some("🦜");
    let x = verify(x, ParrotError::Other("not something"));
    assert_eq!(x, Ok("🦜"));

    let x: Option<&str> = None;
    let x = verify(x, ParrotError::Other("not something"));
    assert_eq!(x, Err(ParrotError::Other("not something")));
}

#[test]
fn test_verify_results() {
    let x: Result<&str, &str> = Ok("🦜");
    let x = verify(x, ParrotError::Other("not ok"));
    assert_eq!(x, Ok("🦜"));

    let x: Result<&str, &str> = Err("fatality");
    let x = verify(x, ParrotError::Other("not ok"));
    assert_eq!(x, Err(ParrotError::Other("not ok")));
}
