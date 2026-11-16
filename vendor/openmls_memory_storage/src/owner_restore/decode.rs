//! Counted decoding for ONLY the schema-audited pinned SDK visitors. This is deliberately not
//! a general serde_json replacement: owned strings/maps/bytes, unknown visitors and in-place
//! container seeds have no permissive path. Input was fully shape-checked under the SAME map
//! guard, then every actual traversal/rescan is still debited from the local work account.
use super::{cursor::Cursor, Account, Error};
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};
use serde::Deserializer as _;

pub(super) fn from_slice<T: DeserializeOwned>(
    bytes: &[u8],
    account: &mut Account,
) -> Result<T, Error> {
    let mut decoder = Json {
        cursor: Cursor::new(bytes, account),
    };
    let result = T::deserialize(&mut decoder)?;
    decoder.cursor.finish()?;
    Ok(result)
}
struct Json<'de, 'a> {
    cursor: Cursor<'de, 'a>,
}
struct Sequence<'s, 'de, 'a> {
    decoder: &'s mut Json<'de, 'a>,
    count: usize,
    index: usize,
    inline: bool,
}
impl<'de> SeqAccess<'de> for Sequence<'_, 'de, '_> {
    type Error = Error;
    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Error> {
        if self.index == self.count {
            return Ok(None);
        }
        if self.index == 0 && !self.inline {
            let element = std::mem::size_of::<S::Value>();
            // All nonempty vectors in the closed SDK profile have nonzero element layout.
            // InPlaceSeed has Value=() but may mutate a large existing T; it MUST NOT inherit
            // this first-element allocation proof. This decoder only calls Deserialize, and
            // additionally refuses nonempty ZST/in-place seeds before invoking their child.
            if element == 0 {
                return Err(Error);
            }
            self.decoder.cursor.account.sequence(self.count, element)?;
        }
        if self.index != 0 {
            self.decoder.cursor.take(b',')?;
        }
        let value = seed.deserialize(&mut *self.decoder)?;
        self.index += 1;
        Ok(Some(value))
    }
    // Pinned serde Vec/Deque first create with_capacity(0). First next_element_seed then
    // prepays ALL growth requests before either the first child or its first container push.
    fn size_hint(&self) -> Option<usize> {
        Some(0)
    }
}
struct Fields<'s, 'de, 'a> {
    decoder: &'s mut Json<'de, 'a>,
    count: usize,
    awaiting_value: bool,
}
impl<'de> MapAccess<'de> for Fields<'_, 'de, '_> {
    type Error = Error;
    fn next_key_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Error> {
        if self.awaiting_value {
            return Err(Error);
        }
        self.decoder.cursor.space()?;
        if self.decoder.cursor.peek()? == Some(b'}') {
            return Ok(None);
        }
        if self.count != 0 {
            self.decoder.cursor.take(b',')?;
        }
        let key = self.decoder.cursor.string()?;
        self.awaiting_value = true;
        seed.deserialize(de::value::BorrowedStrDeserializer::<Error>::new(key))
            .map(Some)
    }
    fn next_value_seed<S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, Error> {
        if !self.awaiting_value {
            return Err(Error);
        }
        self.decoder.cursor.take(b':')?;
        let value = seed.deserialize(&mut *self.decoder)?;
        self.awaiting_value = false;
        self.count += 1;
        Ok(value)
    }
    fn size_hint(&self) -> Option<usize> {
        Some(0)
    }
}
struct Variant<'s, 'de, 'a> {
    decoder: &'s mut Json<'de, 'a>,
    name: &'de str,
    object: bool,
}
impl<'de, 's, 'a> EnumAccess<'de> for Variant<'s, 'de, 'a> {
    type Error = Error;
    type Variant = Self;
    fn variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<(S::Value, Self), Error> {
        // This debit precedes the REAL variant seed. In the pinned serialization helper its
        // VariantIdVisitor::visit_str creates Name(String) via to_string, even for borrowed
        // input. Charging only deserialize_string would miss that concrete copy.
        self.decoder.cursor.account.bytes(self.name.len())?;
        let name = seed.deserialize(de::value::BorrowedStrDeserializer::<Error>::new(self.name))?;
        Ok((name, self))
    }
}
impl<'de> VariantAccess<'de> for Variant<'_, 'de, '_> {
    type Error = Error;
    fn unit_variant(self) -> Result<(), Error> {
        if self.object {
            self.decoder.cursor.word(b"null")?;
            self.decoder.cursor.take(b'}')?;
        }
        Ok(())
    }
    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, Error> {
        if !self.object {
            return Err(Error);
        }
        let result = seed.deserialize(&mut *self.decoder)?;
        self.decoder.cursor.take(b'}')?;
        Ok(result)
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        if !self.object {
            return Err(Error);
        }
        let result = (&mut *self.decoder).deserialize_tuple(len, visitor)?;
        self.decoder.cursor.take(b'}')?;
        Ok(result)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Error> {
        Err(Error)
    }
}
impl<'de> de::Deserializer<'de> for &mut Json<'de, '_> {
    type Error = Error;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.cursor.space()?;
        match self.cursor.peek()? {
            // The only audited untagged visitor is PastEpochDeletionPolicy::Format, and its
            // accepted scalar0 Content has no owned String/Seq/Map capture. Reject those forms.
            Some(b'0'..=b'9') => visitor.visit_u64(self.cursor.number()?),
            Some(b'n') => {
                self.cursor.word(b"null")?;
                visitor.visit_unit()
            }
            Some(b't') => {
                self.cursor.word(b"true")?;
                visitor.visit_bool(true)
            }
            Some(b'f') => {
                self.cursor.word(b"false")?;
                visitor.visit_bool(false)
            }
            _ => Err(Error),
        }
    }
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.cursor.space()?;
        if self.cursor.peek()? == Some(b't') {
            self.cursor.word(b"true")?;
            visitor.visit_bool(true)
        } else {
            self.cursor.word(b"false")?;
            visitor.visit_bool(false)
        }
    }
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u8(u8::try_from(self.cursor.number()?).map_err(|_| Error)?)
    }
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u16(u16::try_from(self.cursor.number()?).map_err(|_| Error)?)
    }
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u32(u32::try_from(self.cursor.number()?).map_err(|_| Error)?)
    }
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u64(self.cursor.number()?)
    }
    fn deserialize_u128<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_i8<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_i16<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_i32<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_i64<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_i128<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_f32<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_f64<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_char<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_str<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_string<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_bytes<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.cursor.space()?;
        if self.cursor.peek()? == Some(b'n') {
            self.cursor.word(b"null")?;
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.cursor.word(b"null")?;
        visitor.visit_unit()
    }
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_unit(visitor)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let count = self.cursor.sequence_count()?;
        self.cursor.take(b'[')?;
        let mut sequence = Sequence {
            decoder: self,
            count,
            index: 0,
            inline: false,
        };
        let result = visitor.visit_seq(&mut sequence)?;
        if sequence.index != count {
            return Err(Error);
        }
        sequence.decoder.cursor.take(b']')?;
        Ok(result)
    }
    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        if len > 12 || self.cursor.sequence_count()? != len {
            return Err(Error);
        }
        self.cursor.take(b'[')?;
        let mut sequence = Sequence {
            decoder: self,
            count: len,
            index: 0,
            inline: true,
        };
        let result = visitor.visit_seq(&mut sequence)?;
        if sequence.index != len {
            return Err(Error);
        }
        sequence.decoder.cursor.take(b']')?;
        Ok(result)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_tuple(len, visitor)
    }
    fn deserialize_map<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.cursor.take(b'{')?;
        let mut fields = Fields {
            decoder: self,
            count: 0,
            awaiting_value: false,
        };
        let result = visitor.visit_map(&mut fields)?;
        if fields.awaiting_value {
            return Err(Error);
        }
        fields.decoder.cursor.take(b'}')?;
        Ok(result)
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.cursor.space()?;
        let object = self.cursor.peek()? == Some(b'{');
        if object {
            self.cursor.take(b'{')?;
        }
        let name = self.cursor.string()?;
        if object {
            self.cursor.take(b':')?;
        }
        visitor.visit_enum(Variant {
            decoder: self,
            name,
            object,
        })
    }
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let identifier = self.cursor.string()?;
        self.cursor.account.bytes(identifier.len())?;
        visitor.visit_borrowed_str(identifier)
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(Error)
    }
    fn is_human_readable(&self) -> bool {
        true
    }
}
