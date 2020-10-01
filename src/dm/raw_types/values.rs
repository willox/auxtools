use super::strings;

#[repr(u8)]
pub enum ValueTag {
    Null, // 0x00
    Turf, // 0x01
    Obj, // 0x02
    Mob, // 0x03
    Area, // 0x04
    Client, // 0x05
    String, // 0x06
}


#[repr(C)]
pub union ValueData {
    string: strings::StringRef,
}

#[repr(C)]
pub struct Value {
    tag: ValueTag,
    data: ValueData,
}