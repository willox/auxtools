use super::raw_types;
use super::GLOBAL_STATE;
use std::ffi::CString;
use std::fmt;
use std::marker::PhantomData;

pub struct Value<'a> {
    pub value: raw_types::values::Value,
    pub phantom: PhantomData<&'a raw_types::values::Value>,
}

impl Value<'_> {
    pub fn null() -> Value<'static> {
        return Value {
            value: raw_types::values::Value {
                tag: raw_types::values::ValueTag::Null,
                data: raw_types::values::ValueData { number: 0.0 },
            },
            phantom: PhantomData {},
        };
    }

    fn get_by_id(&self, name_id: u32) -> Value {
        let val = unsafe {
            (GLOBAL_STATE.get().unwrap().get_variable)(
                self.value.tag as u32,
                std::mem::transmute(self.value.data),
                name_id,
            )
        };
        Self::from(val)
    }

    pub fn get<S: Into<String>>(&self, name: S) -> Option<Value> {
        if let Ok(string) = CString::new(name.into()) {
            let index = unsafe {
                (GLOBAL_STATE.get().unwrap().get_string_id)(string.as_ptr(), true, false, true)
            };
            return Some(self.get_by_id(index));
        }
        None
    }
}

impl fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

pub fn create_value<'a>(
    tag: raw_types::values::ValueTag,
    data: raw_types::values::ValueData,
) -> Value<'a> {
    Value {
        value: raw_types::values::Value { tag, data },
        phantom: PhantomData {},
    }
}

/*
fn value_from_string<'a>(s: &String) -> Value<'a> {
    let mut s = s.clone();
    s.push(0x00 as char);
    let id = unsafe { (GLOBAL_STATE.get().unwrap().get_string_id)(s.as_str(), true, false, true) };
    create_value(
        raw_types::values::ValueTag::String,
        raw_types::values::ValueData {
            string: raw_types::strings::StringRef(id),
        },
    )
}

impl From<&String> for Value<'_> {
    fn from(s: &String) -> Self {
        value_from_string(s)
    }
}

impl From<&str> for Value<'_> {
    fn from(s: &str) -> Self {
        value_from_string(&s.to_owned())
    }
}
*/

impl From<f32> for Value<'_> {
    fn from(num: f32) -> Self {
        create_value(
            raw_types::values::ValueTag::Number,
            raw_types::values::ValueData { number: num },
        )
    }
}

impl From<i32> for Value<'_> {
    fn from(num: i32) -> Self {
        create_value(
            raw_types::values::ValueTag::Number,
            raw_types::values::ValueData { number: num as f32 },
        )
    }
}

impl From<u32> for Value<'_> {
    fn from(num: u32) -> Self {
        create_value(
            raw_types::values::ValueTag::Number,
            raw_types::values::ValueData { number: num as f32 },
        )
    }
}

impl From<bool> for Value<'_> {
    fn from(b: bool) -> Self {
        create_value(
            raw_types::values::ValueTag::Number,
            raw_types::values::ValueData {
                number: if b { 1.0 } else { 0.0 },
            },
        )
    }
}

impl From<raw_types::values::Value> for Value<'_> {
    fn from(v: raw_types::values::Value) -> Self {
        create_value(v.tag, v.data)
    }
}
