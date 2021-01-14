#[derive(Debug, PartialEq)]
pub enum Error {
    ConstOverflow,
    ArgOverflow,
    ArgCount(u16, u16),
    LetValueCount(u16),
    StackSizeOverflow,
    Type(Type, Type),
    UnknownOpcode(u8),
    VoidFunction,
    VoidVariable,
}

#[derive(Debug, PartialEq)]
pub enum Type {
    Int,
    True,
    Nil,
    Cons,
    String,
    Symbol,
    Float,
    Void,
    Marker,
    Func,
    Number,
    List,
}

pub type Result<T> = std::result::Result<T, Error>;
