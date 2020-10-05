use super::raw_types;
use super::string;
use super::GLOBAL_STATE;
use std::ffi::CString;
use std::fmt;
use std::marker::PhantomData;

#[derive(Copy, Clone)]
pub struct Value<'a> {
    pub value: raw_types::values::Value,
    pub phantom: PhantomData<&'a raw_types::values::Value>,
}

impl<'b> Value<'b> {
    pub unsafe fn new<'a>(
        tag: raw_types::values::ValueTag,
        data: raw_types::values::ValueData,
    ) -> Value<'a> {
        Value {
            value: raw_types::values::Value { tag, data },
            phantom: PhantomData {},
        }
    }

    pub fn null() -> Value<'static> {
        return Value {
            value: raw_types::values::Value {
                tag: raw_types::values::ValueTag::Null,
                data: raw_types::values::ValueData { number: 0.0 },
            },
            phantom: PhantomData {},
        };
    }

    fn get_by_id(&self, name_id: u32) -> Value<'b> {
        let val = unsafe { (GLOBAL_STATE.get().unwrap().get_variable)(self.value, name_id) };
        unsafe { Self::from_raw(val) }
    }

    fn set_by_id(&self, name_id: u32, new_value: raw_types::values::Value) {
        unsafe { (GLOBAL_STATE.get().unwrap().set_variable)(self.value, name_id, new_value) }
    }

    pub fn get<S: Into<String>>(&self, name: S) -> Option<Value<'b>> {
        if let Ok(string) = CString::new(name.into()) {
            let index = unsafe {
                (GLOBAL_STATE.get().unwrap().get_string_id)(string.as_ptr(), true, false, true)
            };
            return Some(self.get_by_id(index));
        }
        None
    }

    pub fn set<S: Into<String>, V: raw_types::values::IntoRawValue>(&self, name: S, new_value: &V) {
        if let Ok(string) = CString::new(name.into()) {
            let index = unsafe {
                (GLOBAL_STATE.get().unwrap().get_string_id)(string.as_ptr(), true, false, true)
            };
            self.set_by_id(index, unsafe { new_value.into_raw_value() });
        }
    }

    // blah blah lifetime is not verified with this so use at your peril
    pub unsafe fn from_raw(v: raw_types::values::Value) -> Self {
        Value::new(v.tag, v.data)
    }
}

impl fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl From<f32> for Value<'_> {
    fn from(num: f32) -> Self {
        unsafe {
            Value::new(
                raw_types::values::ValueTag::Number,
                raw_types::values::ValueData { number: num },
            )
        }
    }
}

impl From<i32> for Value<'_> {
    fn from(num: i32) -> Self {
        unsafe {
            Value::new(
                raw_types::values::ValueTag::Number,
                raw_types::values::ValueData { number: num as f32 },
            )
        }
    }
}

impl From<u32> for Value<'_> {
    fn from(num: u32) -> Self {
        unsafe {
            Value::new(
                raw_types::values::ValueTag::Number,
                raw_types::values::ValueData { number: num as f32 },
            )
        }
    }
}

impl From<bool> for Value<'_> {
    fn from(b: bool) -> Self {
        unsafe {
            Value::new(
                raw_types::values::ValueTag::Number,
                raw_types::values::ValueData {
                    number: if b { 1.0 } else { 0.0 },
                },
            )
        }
    }
}

impl raw_types::values::IntoRawValue for Value<'_> {
    unsafe fn into_raw_value(&self) -> raw_types::values::Value {
        self.value
    }
}
