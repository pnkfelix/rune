use anyhow::Result;
use fn_macros::defun;

use crate::core::object::Gc;

#[defun]
fn unibyte_string(bytes: &[Gc<i64>]) -> Result<Vec<u8>> {
    let unibyte: Result<Vec<u8>, _> = bytes.iter().map(|x| u8::try_from(x.untag())).collect();
    Ok(unibyte?)
}
