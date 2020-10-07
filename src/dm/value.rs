use super::raw_types;
use super::string;
use super::GLOBAL_STATE;
use crate::raw_types::values::IntoRawValue;
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
        // unsafe { (GLOBAL_STATE.get().unwrap().inc_ref_count)(val) }
        unsafe { Self::from_raw(val) }
    }

    fn set_by_id(&self, name_id: u32, new_value: raw_types::values::Value) {
        unsafe { (GLOBAL_STATE.get().unwrap().set_variable)(self.value, name_id, new_value) }
    }

    pub fn get<S: Into<string::StringRef>>(&self, name: S) -> Option<Value<'b>> {
        unsafe { Some(self.get_by_id((*name.into().internal).this.0)) }
    }

    pub fn get_float<S: Into<string::StringRef>>(&self, name: S) -> Option<f32> {
        let var = self.get(name).unwrap();
        match var.value.tag {
            raw_types::values::ValueTag::Number => Some(unsafe { var.value.data.number }),
            _ => None,
        }
    }

    pub fn get_string<S: Into<string::StringRef>>(&self, name: S) -> Option<String> {
        let var = self.get(name).unwrap();
        match var.value.tag {
            raw_types::values::ValueTag::String => {
                let id = unsafe { var.value.data.id };
                let s = string::StringRef::from_id(id);
                Some(s.into())
            }
            _ => None,
        }
    }

    pub fn set<S: Into<string::StringRef>, V: raw_types::values::IntoRawValue>(
        &self,
        name: S,
        new_value: &V,
    ) {
        unsafe {
            self.set_by_id((*name.into().internal).this.0, new_value.into_raw_value());
        }
    }

    pub fn call<S: AsRef<str>, R: Into<RawValueVector> + Default>(
        &self,
        procname: S,
        args: Option<R>,
    ) -> Value<'b> {
        unsafe {
            let procname = String::from(procname.as_ref()).replace("_", " ");
            let mut args: Vec<raw_types::values::Value> = args.unwrap_or_default().into().0;

            let result = (GLOBAL_STATE.get().unwrap().call_datum_proc_by_name)(
                Value::null().into_raw_value(),
                2,
                (*string::StringRef::from(procname).internal).this,
                self.into_raw_value(),
                args.as_mut_ptr(),
                args.len(),
                0,
                0,
            );
            Value::from_raw(result)
        }
    }

    // blah blah lifetime is not verified with this so use at your peril
    pub unsafe fn from_raw(v: raw_types::values::Value) -> Self {
        Value::new(v.tag, v.data)
    }
}

pub enum EitherValue<'a> {
    Val(Value<'a>),
    Str(string::StringRef),
}

pub struct RawValueVector(pub Vec<raw_types::values::Value>);

impl<'a> From<Vec<EitherValue<'a>>> for RawValueVector {
    fn from(v: Vec<EitherValue>) -> Self {
        RawValueVector(unsafe {
            v.iter()
                .map(|v| match v {
                    EitherValue::Val(v) => v.into_raw_value(),
                    EitherValue::Str(s) => s.into_raw_value(),
                })
                .collect()
        })
    }
}

impl Default for RawValueVector {
    fn default() -> Self {
        Self(vec![])
    }
}

impl<'a> From<Value<'a>> for EitherValue<'a> {
    fn from(v: Value<'a>) -> Self {
        EitherValue::Val(v)
    }
}

impl<'a> From<string::StringRef> for EitherValue<'a> {
    fn from(s: string::StringRef) -> Self {
        EitherValue::Str(s)
    }
}

impl<'a> From<String> for EitherValue<'a> {
    fn from(s: String) -> Self {
        EitherValue::Str(string::StringRef::from(s))
    }
}

impl<'a> From<&str> for EitherValue<'a> {
    fn from(s: &str) -> Self {
        EitherValue::Str(string::StringRef::from(s))
    }
}

impl<'a> From<f32> for EitherValue<'a> {
    fn from(f: f32) -> Self {
        EitherValue::Val(Value::from(f))
    }
}

impl<'a> From<Vec<string::StringRef>> for RawValueVector {
    fn from(v: Vec<string::StringRef>) -> Self {
        RawValueVector(unsafe { v.iter().map(|v| v.into_raw_value()).collect() })
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

impl raw_types::values::IntoRawValue for EitherValue<'_> {
    unsafe fn into_raw_value(&self) -> raw_types::values::Value {
        match self {
            EitherValue::Val(v) => v.into_raw_value(),
            EitherValue::Str(s) => s.into_raw_value(),
        }
    }
}
