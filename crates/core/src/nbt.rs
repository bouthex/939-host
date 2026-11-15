//! Lectura de datos de Minecraft: SNBT (texto que responde `/data get`) y NBT binario
//! (archivos `playerdata/*.dat`). Todo se convierte a `serde_json::Value`.
//!
//! Los números conservan su valor; el tipo exacto (byte, short, float…) no hace falta para
//! mostrar la información.

use serde_json::{Map, Number, Value};

// ───────── SNBT ─────────

pub fn parse_snbt(s: &str) -> Result<Value, String> {
    let mut p = Snbt { b: s.as_bytes(), i: 0 };
    p.ws();
    let v = p.value()?;
    Ok(v)
}

struct Snbt<'a> { b: &'a [u8], i: usize }

impl<'a> Snbt<'a> {
    fn ws(&mut self) { while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() { self.i += 1; } }
    fn peek(&self) -> Option<u8> { self.b.get(self.i).copied() }
    fn eat(&mut self, c: u8) -> bool { self.ws(); if self.peek() == Some(c) { self.i += 1; true } else { false } }
    fn err<T>(&self, m: &str) -> Result<T, String> { Err(format!("{m} en la posición {}", self.i)) }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.peek() {
            Some(b'{') => self.compound(),
            Some(b'[') => self.list(),
            Some(b'"') | Some(b'\'') => Ok(Value::String(self.quoted()?)),
            Some(_) => self.bare(),
            None => self.err("fin inesperado"),
        }
    }

    fn compound(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut m = Map::new();
        if self.eat(b'}') { return Ok(Value::Object(m)); }
        loop {
            self.ws();
            let key = match self.peek() {
                Some(b'"') | Some(b'\'') => self.quoted()?,
                _ => self.bare_token(),
            };
            if key.is_empty() { return self.err("clave vacía"); }
            if !self.eat(b':') { return self.err("falta «:»"); }
            let v = self.value()?;
            m.insert(key, v);
            if self.eat(b',') { continue; }
            if self.eat(b'}') { break; }
            return self.err("falta «}»");
        }
        Ok(Value::Object(m))
    }

    fn list(&mut self) -> Result<Value, String> {
        self.i += 1;
        // Arreglos tipados: [I; 1, 2], [B; …], [L; …]
        self.ws();
        if self.b.len() > self.i + 1 && matches!(self.b[self.i], b'I' | b'B' | b'L') && self.b[self.i + 1] == b';' { self.i += 2; }
        let mut out = vec![];
        if self.eat(b']') { return Ok(Value::Array(out)); }
        loop {
            out.push(self.value()?);
            if self.eat(b',') { continue; }
            if self.eat(b']') { break; }
            return self.err("falta «]»");
        }
        Ok(Value::Array(out))
    }

    fn quoted(&mut self) -> Result<String, String> {
        let q = self.b[self.i];
        self.i += 1;
        let mut out: Vec<u8> = vec![];
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == b'\\' {
                match self.peek() {
                    Some(b'n') => { out.push(b'\n'); self.i += 1; }
                    Some(b't') => { out.push(b'\t'); self.i += 1; }
                    Some(e) => { out.push(e); self.i += 1; }
                    None => break,
                }
            } else if c == q {
                return String::from_utf8(out).map_err(|_| "texto inválido".into());
            } else {
                out.push(c);
            }
        }
        self.err("texto sin cerrar")
    }

    fn bare_token(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+') { self.i += 1; } else { break; }
        }
        String::from_utf8_lossy(&self.b[start..self.i]).into_owned()
    }

    fn bare(&mut self) -> Result<Value, String> {
        let t = self.bare_token();
        if t.is_empty() { return self.err("valor inválido"); }
        Ok(bare_value(&t))
    }
}

fn bare_value(t: &str) -> Value {
    match t {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    let (num, suf) = match t.chars().last() {
        Some(c) if "bBsSlLfFdD".contains(c) && t.len() > 1 => (&t[..t.len() - 1], Some(c.to_ascii_lowercase())),
        _ => (t, None),
    };
    let is_float = suf.map(|c| c == 'f' || c == 'd').unwrap_or(false) || num.contains('.') || num.contains('e') || num.contains('E');
    if is_float {
        if let Ok(f) = num.parse::<f64>() { if let Some(n) = Number::from_f64(f) { return Value::Number(n); } }
    } else if let Ok(i) = num.parse::<i64>() {
        return Value::Number(i.into());
    }
    Value::String(t.to_string())
}

// ───────── NBT binario ─────────

/// Lee un archivo NBT (comprimido con gzip o no).
pub fn read_nbt_file(path: &std::path::Path) -> Result<Value, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    let data = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut d = flate2::read::GzDecoder::new(&raw[..]);
        let mut out = vec![];
        std::io::Read::read_to_end(&mut d, &mut out).map_err(|e| e.to_string())?;
        out
    } else { raw };
    parse_nbt(&data)
}

pub fn parse_nbt(data: &[u8]) -> Result<Value, String> {
    let mut r = Nbt { b: data, i: 0 };
    let tag = r.u8()?;
    if tag != 10 { return Err("el archivo no empieza con un compuesto".into()); }
    let _name = r.string()?;
    r.payload(10, 0)
}

struct Nbt<'a> { b: &'a [u8], i: usize }

impl<'a> Nbt<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.i + n > self.b.len() { return Err("archivo NBT truncado".into()); }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    fn i16(&mut self) -> Result<i16, String> { let s = self.take(2)?; Ok(i16::from_be_bytes([s[0], s[1]])) }
    fn i32(&mut self) -> Result<i32, String> { let s = self.take(4)?; Ok(i32::from_be_bytes(s.try_into().unwrap())) }
    fn i64(&mut self) -> Result<i64, String> { let s = self.take(8)?; Ok(i64::from_be_bytes(s.try_into().unwrap())) }
    fn string(&mut self) -> Result<String, String> {
        let n = self.i16()? as u16 as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn len(&mut self) -> Result<usize, String> {
        let n = self.i32()?;
        if n < 0 || n as usize > self.b.len() { return Err("longitud inválida".into()); }
        Ok(n as usize)
    }
    fn payload(&mut self, tag: u8, depth: usize) -> Result<Value, String> {
        if depth > 64 { return Err("datos demasiado anidados".into()); }
        let f = |x: f64| Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null);
        Ok(match tag {
            1 => Value::Number((self.u8()? as i8 as i64).into()),
            2 => Value::Number((self.i16()? as i64).into()),
            3 => Value::Number((self.i32()? as i64).into()),
            4 => Value::Number(self.i64()?.into()),
            5 => { let s = self.take(4)?; f(f32::from_be_bytes(s.try_into().unwrap()) as f64) }
            6 => { let s = self.take(8)?; f(f64::from_be_bytes(s.try_into().unwrap())) }
            7 => { let n = self.len()?; Value::Array(self.take(n)?.iter().map(|b| Value::Number((*b as i8 as i64).into())).collect()) }
            8 => Value::String(self.string()?),
            9 => {
                let t = self.u8()?;
                let n = self.len()?;
                let mut out = Vec::with_capacity(n.min(4096));
                for _ in 0..n { out.push(self.payload(t, depth + 1)?); }
                Value::Array(out)
            }
            10 => {
                let mut m = Map::new();
                loop {
                    let t = self.u8()?;
                    if t == 0 { break; }
                    let k = self.string()?;
                    let v = self.payload(t, depth + 1)?;
                    m.insert(k, v);
                }
                Value::Object(m)
            }
            11 => { let n = self.len()?; let mut out = vec![]; for _ in 0..n { out.push(Value::Number((self.i32()? as i64).into())); } Value::Array(out) }
            12 => { let n = self.len()?; let mut out = vec![]; for _ in 0..n { out.push(Value::Number(self.i64()?.into())); } Value::Array(out) }
            _ => return Err(format!("etiqueta NBT desconocida {tag}")),
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn parses_snbt_player_reply() {
        let s = r#"{Health: 17.0f, foodLevel: 14, XpLevel: 17, XpP: 0.62f, Dimension: "minecraft:overworld", Pos: [412.3d, 68.0d, -1290.7d], UUID: [I; 1, -2, 3, 4], Inventory: [{Slot: 0b, id: "minecraft:diamond_sword", count: 1, components: {"minecraft:damage": 280, "minecraft:enchantments": {"minecraft:sharpness": 4}}}], playerGameType: 0, name: 'Lu\'s', flag: 1b, big: 12L, neg: -5.5e2}"#;
        let v = parse_snbt(s).unwrap();
        assert_eq!(v["Health"].as_f64(), Some(17.0));
        assert_eq!(v["foodLevel"].as_i64(), Some(14));
        assert_eq!(v["Pos"][2].as_f64(), Some(-1290.7));
        assert_eq!(v["UUID"][1].as_i64(), Some(-2));
        assert_eq!(v["Inventory"][0]["id"], "minecraft:diamond_sword");
        assert_eq!(v["Inventory"][0]["components"]["minecraft:damage"].as_i64(), Some(280));
        assert_eq!(v["name"], "Lu's");
        assert_eq!(v["neg"].as_f64(), Some(-550.0));
    }

    /// Escritor mínimo de NBT para las pruebas.
    pub fn write_nbt(v: &Value) -> Vec<u8> {
        fn s(out: &mut Vec<u8>, x: &str) { out.extend((x.len() as u16).to_be_bytes()); out.extend(x.as_bytes()); }
        fn tag(v: &Value) -> u8 { match v { Value::Object(_) => 10, Value::Array(_) => 9, Value::String(_) => 8, Value::Number(n) if n.is_f64() => 6, Value::Number(_) => 3, Value::Bool(_) => 1, Value::Null => 8 } }
        fn p(out: &mut Vec<u8>, v: &Value) {
            match v {
                Value::Object(m) => { for (k, x) in m { out.push(tag(x)); s(out, k); p(out, x); } out.push(0); }
                Value::Array(a) => { out.push(a.first().map(tag).unwrap_or(0)); out.extend((a.len() as i32).to_be_bytes()); for x in a { p(out, x); } }
                Value::String(x) => s(out, x),
                Value::Number(n) if n.is_f64() => out.extend(n.as_f64().unwrap().to_be_bytes()),
                Value::Number(n) => out.extend((n.as_i64().unwrap() as i32).to_be_bytes()),
                Value::Bool(b) => out.push(*b as u8),
                Value::Null => s(out, ""),
            }
        }
        let mut out = vec![10];
        s(&mut out, "");
        p(&mut out, v);
        out
    }

    #[test]
    fn binary_nbt_roundtrip() {
        let v = serde_json::json!({"Health": 20.0, "Pos": [1.5, 64.0, -3.25], "Inventory": [{"id": "minecraft:stone", "count": 64, "Slot": 3}], "Dimension": "minecraft:the_nether"});
        let parsed = parse_nbt(&write_nbt(&v)).unwrap();
        assert_eq!(parsed["Pos"][2].as_f64(), Some(-3.25));
        assert_eq!(parsed["Inventory"][0]["count"].as_i64(), Some(64));
        assert_eq!(parsed["Dimension"], "minecraft:the_nether");
        assert!(parse_nbt(&[10, 0]).is_err());
    }
}
