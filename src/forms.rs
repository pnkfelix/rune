use crate::object::{Function, Object, Symbol, NIL};
use fn_macros::lisp_fn;

#[lisp_fn]
pub fn defalias(symbol: Symbol, definition: Function) -> Symbol {
    symbol.set_func(definition);
    symbol
}

#[lisp_fn]
pub fn progn<'obj>(forms: &[Object<'obj>]) -> Object<'obj> {
    match forms.last() {
        Some(form) => *form,
        None => NIL,
    }
}

defsubr!(defalias, progn);
