use gw_types::{packed, prelude::*};
use std::fmt::{self, Debug};
use std::hash::{Hash, Hasher};

#[derive(Clone)]
pub struct Byte65(pub [u8; 65]);

impl Default for Byte65 {
    fn default() -> Self {
        Byte65([0u8; 65])
    }
}

impl PartialEq for Byte65 {
    fn eq(&self, other: &Byte65) -> bool {
        &self.0[..] == &other.0[..]
    }
}

impl Eq for Byte65 {}

impl Hash for Byte65 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.0);
    }
}

impl Debug for Byte65 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Byte65 {
    pub fn new(inner: [u8; 65]) -> Self {
        Byte65(inner)
    }
}

impl From<packed::Signature> for Byte65 {
    fn from(packed: packed::Signature) -> Self {
        let mut inner: [u8; 65] = [0u8; 65];
        inner.copy_from_slice(&packed.raw_data());
        Byte65(inner)
    }
}

impl From<Byte65> for packed::Signature {
    fn from(json: Byte65) -> Self {
        Self::from_slice(&json.0).expect("impossible: fail to read inner array")
    }
}

struct Byte32Visitor;

impl<'b> serde::de::Visitor<'b> for Byte32Visitor {
    type Value = Byte65;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a 0x-prefixed hex string")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v.len() < 2 || &v.as_bytes()[0..2] != b"0x" || v.len() != 66 {
            return Err(E::invalid_value(serde::de::Unexpected::Str(v), &self));
        }
        let decoded_bytes =
            hex::decode(&v.as_bytes()[2..]).map_err(|e| E::custom(format_args!("{:?}", e)))?;
        let mut buffer = [0u8; 65]; // we checked length
        buffer.copy_from_slice(&decoded_bytes);
        Ok(Byte65(buffer))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&v)
    }
}

impl serde::Serialize for Byte65 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut buffer = [0u8; 132];
        buffer[0] = b'0';
        buffer[1] = b'x';
        let encoded_bytes = hex::encode(&self.0);
        buffer.copy_from_slice(encoded_bytes.as_bytes());
        serializer.serialize_str(unsafe { ::std::str::from_utf8_unchecked(&buffer) })
    }
}

impl<'de> serde::Deserialize<'de> for Byte65 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(Byte32Visitor)
    }
}
