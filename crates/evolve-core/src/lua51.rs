//! Read the literal tables a compiled Lua 5.1 (`\x1bLuaQ`) or Stage 2 (`\x1bJee`) chunk assigns to globals.
//!
//! Legacy actor and creature scripts are stock Lua 5.1 bytecode, and Stage 2
//! actor and creature scripts are a custom `\x1bJee` bytecode with IEEE float32
//! numbers and alternate opcode assignments. In both, the main chunk builds
//! each subtype as a table literal and stores it in a global.
//! The script is never executed: the main chunk's straight-line code is
//! followed, and only values loaded from constants or built from tables are
//! kept. Any branch target, jump, call or computed value makes a register
//! unknown, so a field is reported only when its key and value are authored
//! constants. Nested functions are runtime code and are not read.

use std::collections::{BTreeSet, HashMap};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_FUNCTIONS: usize = 4096;
const MAX_ITEMS: usize = 1 << 20;
const MAX_DEPTH: usize = 64;
const MAX_TABLE_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Table(Table),
    /// A global read by name, e.g. `baseType = GoliathA1`.
    Global(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// String-keyed fields in authored order; a repeated key keeps its last value.
    pub fields: Vec<(String, Value)>,
    /// Array part filled by the table constructor's list items.
    pub items: Vec<Value>,
}

impl Table {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn table(&self, key: &str) -> Option<&Table> {
        match self.get(key)? {
            Value::Table(table) => Some(table),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }

    /// Every scalar as `path · key` and display text, for read-only preview.
    pub fn flatten(&self, prefix: &str, out: &mut Vec<(String, String)>) {
        for (key, value) in &self.fields {
            flatten_value(&format!("{prefix} / {key}"), value, out);
        }
        for (index, value) in self.items.iter().enumerate() {
            flatten_value(&format!("{prefix} / [{}]", index + 1), value, out);
        }
    }
}

fn flatten_value(label: &str, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Table(table) => table.flatten(label, out),
        Value::Str(text) => out.push((label.to_owned(), text.clone())),
        Value::Num(number) => out.push((label.to_owned(), number.to_string())),
        Value::Bool(flag) => out.push((label.to_owned(), flag.to_string())),
        Value::Global(name) => out.push((label.to_owned(), name.clone())),
    }
}

pub fn is_bytecode(bytes: &[u8]) -> bool {
    is_lua51(bytes) || is_jee(bytes)
}

pub fn is_lua51(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x1bLuaQ")
}

pub fn is_jee(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x1bJee")
}

struct Function {
    code: Vec<u32>,
    constants: Vec<Constant>,
    upvalue_counts: Vec<usize>,
}

#[derive(Clone)]
enum Constant {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    size_t: usize,
    number: usize,
    integral: bool,
    functions: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("Truncated Lua bytecode")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn uint(&mut self, width: usize) -> Result<u64, String> {
        let mut value = [0u8; 8];
        value[..width].copy_from_slice(self.take(width)?);
        Ok(u64::from_le_bytes(value))
    }

    fn count(&mut self) -> Result<usize, String> {
        let count = self.uint(4)? as usize;
        if count > MAX_ITEMS || count > self.bytes.len() {
            return Err("Lua bytecode count exceeds its size".into());
        }
        Ok(count)
    }

    fn string(&mut self) -> Result<Option<String>, String> {
        let length = self.uint(self.size_t)? as usize;
        if length == 0 {
            return Ok(None);
        }
        let raw = self.take(length)?;
        let (text, terminator) = raw.split_at(length - 1);
        if terminator != [0] {
            return Err("Lua string is not terminated".into());
        }
        Ok(Some(String::from_utf8_lossy(text).into_owned()))
    }

    fn number(&mut self) -> Result<f64, String> {
        let layout = (self.integral, self.number);
        let raw = self.take(layout.1)?;
        Ok(match layout {
            (false, 4) => f32::from_le_bytes(raw.try_into().unwrap()) as f64,
            (false, _) => f64::from_le_bytes(raw.try_into().unwrap()),
            (true, 4) => i32::from_le_bytes(raw.try_into().unwrap()) as f64,
            (true, _) => i64::from_le_bytes(raw.try_into().unwrap()) as f64,
        })
    }

    /// Read one function prototype; returns its own code and constants, and
    /// only the upvalue count of each nested prototype.
    fn function(&mut self, depth: usize) -> Result<(Function, usize), String> {
        self.functions += 1;
        if depth > MAX_DEPTH || self.functions > MAX_FUNCTIONS {
            return Err("Lua bytecode nests too deeply".into());
        }
        self.string()?;
        self.take(8)?;
        let header = self.take(4)?;
        let upvalues = header[0] as usize;
        let code = (0..self.count()?)
            .map(|_| self.uint(4).map(|word| word as u32))
            .collect::<Result<Vec<_>, _>>()?;
        let mut constants = Vec::new();
        for _ in 0..self.count()? {
            constants.push(match self.take(1)?[0] {
                0 => Constant::Nil,
                1 => Constant::Bool(self.take(1)?[0] != 0),
                3 => Constant::Num(self.number()?),
                4 => Constant::Str(self.string()?.unwrap_or_default()),
                _ => return Err("Unknown Lua constant type".into()),
            });
        }
        let mut upvalue_counts = Vec::new();
        for _ in 0..self.count()? {
            upvalue_counts.push(self.function(depth + 1)?.1);
        }
        for _ in 0..self.count()? {
            self.take(4)?;
        }
        for _ in 0..self.count()? {
            self.string()?;
            self.take(8)?;
        }
        for _ in 0..self.count()? {
            self.string()?;
        }
        Ok((
            Function {
                code,
                constants,
                upvalue_counts,
            },
            upvalues,
        ))
    }

    /// Read one Stage 2 (`\x1bJee`) prototype. In Jee bytecode, constants and
    /// nested prototypes precede code, followed by lineinfo, locals and upvalues.
    fn function_jee(&mut self, depth: usize) -> Result<(Function, usize), String> {
        self.functions += 1;
        if depth > MAX_DEPTH || self.functions > MAX_FUNCTIONS {
            return Err("Lua bytecode nests too deeply".into());
        }
        self.string()?;
        self.take(8)?;
        let header = self.take(4)?;
        let upvalues = header[0] as usize;
        let mut constants = Vec::new();
        for _ in 0..self.count()? {
            constants.push(match self.take(1)?[0] {
                0 => Constant::Nil,
                1 => Constant::Bool(self.take(1)?[0] != 0),
                3 => Constant::Num(self.number()?),
                4 => Constant::Str(self.string()?.unwrap_or_default()),
                _ => return Err("Unknown Lua constant type".into()),
            });
        }
        let mut upvalue_counts = Vec::new();
        for _ in 0..self.count()? {
            upvalue_counts.push(self.function_jee(depth + 1)?.1);
        }
        let code = (0..self.count()?)
            .map(|_| self.uint(4).map(|word| word as u32))
            .collect::<Result<Vec<_>, _>>()?;
        for _ in 0..self.count()? {
            self.take(4)?;
        }
        for _ in 0..self.count()? {
            self.string()?;
            self.take(8)?;
        }
        for _ in 0..self.count()? {
            self.string()?;
        }
        Ok((
            Function {
                code,
                constants,
                upvalue_counts,
            },
            upvalues,
        ))
    }
}

fn parse(bytes: &[u8]) -> Result<Function, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Lua bytecode is too large".into());
    }
    // ESC "Lua", version 5.1, official format, little-endian, 4-byte int and
    // instruction; size_t and lua_Number may be 4 or 8 bytes.
    let header = bytes.get(..12).ok_or("Truncated Lua bytecode")?;
    if !is_lua51(bytes)
        || header[5] != 0
        || header[6] != 1
        || header[7] != 4
        || !matches!(header[8], 4 | 8)
        || header[9] != 4
        || !matches!(header[10], 4 | 8)
        || header[11] > 1
    {
        return Err("Unsupported Lua 5.1 bytecode layout".into());
    }
    let mut reader = Reader {
        bytes,
        at: 12,
        size_t: header[8] as usize,
        number: header[10] as usize,
        integral: header[11] == 1,
        functions: 0,
    };
    let (main, _) = reader.function(0)?;
    if reader.at != bytes.len() {
        return Err("Unexpected data after Lua bytecode".into());
    }
    Ok(main)
}

fn parse_jee(bytes: &[u8]) -> Result<Function, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Lua bytecode is too large".into());
    }
    if !is_jee(bytes) {
        return Err("Unsupported Stage 2 bytecode layout".into());
    }
    let mut reader = Reader {
        bytes,
        at: 4,
        size_t: 8,
        number: 4,
        integral: false,
        functions: 0,
    };
    let (main, _) = reader.function_jee(0)?;
    if reader.at != bytes.len() {
        return Err("Unexpected data after Lua bytecode".into());
    }
    Ok(main)
}

#[derive(Clone)]
enum Known {
    Const(Constant),
    Table(usize),
    Global(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum BytecodeFormat {
    Lua51,
    Jee,
}

enum DecodedOp {
    Move { a: usize, b: usize },
    LoadK { a: usize, bx: usize },
    LoadBool { a: usize, b: usize, skip: bool },
    LoadNil { a: usize, b: usize },
    GetGlobal { a: usize, bx: usize },
    GetTable { a: usize, b: usize, c: usize },
    SetGlobal { a: usize, bx: usize },
    SetTable { a: usize, b: usize, c: usize },
    NewTable { a: usize },
    SelfOp { a: usize },
    SetList { a: usize, b: usize, c: usize },
    Closure { a: usize, bx: usize },
    Jump { sbx: i64 },
    SkipNext,
    SkipNextClear,
    Clear,
    Invalidate(usize),
    Noop,
}

const LFIELDS_PER_FLUSH: usize = 50;

fn decode_instruction(format: BytecodeFormat, word: u32) -> DecodedOp {
    let raw_op = word & 63;
    let a = ((word >> 6) & 255) as usize;
    let c = ((word >> 14) & 511) as usize;
    let b = ((word >> 23) & 511) as usize;
    let bx = (word >> 14) as usize;
    let sbx = ((word >> 14) as i64) - 131_071;

    match format {
        BytecodeFormat::Lua51 => match raw_op {
            0 => DecodedOp::Move { a, b },
            1 => DecodedOp::LoadK { a, bx },
            2 => DecodedOp::LoadBool { a, b, skip: c != 0 },
            3 => DecodedOp::LoadNil { a, b },
            5 => DecodedOp::GetGlobal { a, bx },
            6 => DecodedOp::GetTable { a, b, c },
            7 => DecodedOp::SetGlobal { a, bx },
            8 => DecodedOp::Noop, // SETUPVAL
            9 => DecodedOp::SetTable { a, b, c },
            10 => DecodedOp::NewTable { a },
            11 => DecodedOp::SelfOp { a },
            22 => DecodedOp::Jump { sbx },
            23..=27 => DecodedOp::SkipNext, // EQ, LT, LE, TEST, TESTSET
            31 | 32 => DecodedOp::Jump { sbx }, // FORLOOP, FORPREP
            33 => DecodedOp::SkipNextClear, // TFORLOOP
            34 => DecodedOp::SetList { a, b, c },
            35 => DecodedOp::Noop, // CLOSE
            36 => DecodedOp::Closure { a, bx },
            28..=30 | 37 => DecodedOp::Clear, // CALL, TAILCALL, RETURN, VARARG
            _ => DecodedOp::Invalidate(a),
        },
        BytecodeFormat::Jee => match raw_op {
            2 => DecodedOp::GetTable { a, b, c },
            3 => DecodedOp::LoadBool { a, b, skip: c != 0 },
            7 => DecodedOp::SetTable { a, b, c },
            8 => DecodedOp::Clear,         // CALL
            10 => DecodedOp::Jump { sbx }, // FORPREP
            11 => DecodedOp::LoadK { a, bx },
            12 => DecodedOp::SetGlobal { a, bx },
            13 => DecodedOp::Move { a, b },
            16 => DecodedOp::LoadNil { a, b },
            17 | 25 | 37 => DecodedOp::SkipNext, // TEST, comparisons
            19 => DecodedOp::NewTable { a },
            21 | 27 => DecodedOp::Jump { sbx }, // JMP, FORLOOP
            23 => DecodedOp::Clear,             // RETURN
            28 => DecodedOp::GetGlobal { a, bx },
            29 => DecodedOp::Closure { a, bx },
            30 => DecodedOp::SelfOp { a },
            33 => DecodedOp::SetList { a, b, c },
            35 => DecodedOp::SkipNextClear, // TFORLOOP
            _ => DecodedOp::Invalidate(a),
        },
    }
}

/// Globals whose value is a table literal built by the main chunk, in the
/// order they are assigned. A later assignment to the same name replaces it.
pub fn authored_globals(bytes: &[u8]) -> Result<Vec<(String, Table)>, String> {
    let (format, main) = if is_lua51(bytes) {
        (BytecodeFormat::Lua51, parse(bytes)?)
    } else if is_jee(bytes) {
        (BytecodeFormat::Jee, parse_jee(bytes)?)
    } else {
        return Err("Unsupported Lua bytecode magic".into());
    };
    let code = &main.code;
    let constant = |index: usize| -> Result<Constant, String> {
        main.constants
            .get(index)
            .cloned()
            .ok_or_else(|| "Lua constant index out of range".to_owned())
    };
    let mut targets = BTreeSet::new();
    for (pc, word) in code.iter().enumerate() {
        match decode_instruction(format, *word) {
            DecodedOp::Jump { sbx } => {
                targets.insert(pc as i64 + 1 + sbx);
            }
            DecodedOp::SkipNext | DecodedOp::SkipNextClear => {
                targets.insert(pc as i64 + 2);
            }
            DecodedOp::LoadBool { skip: true, .. } => {
                targets.insert(pc as i64 + 2);
            }
            _ => {}
        }
    }
    // Table values are arena indices so aliases see the same writes.
    let mut table_fields: Vec<Vec<(String, Known)>> = Vec::new();
    let mut table_items: Vec<Vec<Known>> = Vec::new();
    let mut registers: HashMap<usize, Known> = HashMap::new();
    let mut globals: Vec<(String, usize)> = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        if targets.contains(&(pc as i64)) {
            registers.clear();
        }
        let word = code[pc];
        pc += 1;
        let op = decode_instruction(format, word);
        let rk =
            |registers: &HashMap<usize, Known>, operand: usize| -> Result<Option<Known>, String> {
                Ok(if operand >= 256 {
                    Some(Known::Const(constant(operand - 256)?))
                } else {
                    registers.get(&operand).cloned()
                })
            };
        match op {
            DecodedOp::Move { a, b } => match registers.get(&b).cloned() {
                Some(value) => {
                    registers.insert(a, value);
                }
                None => {
                    registers.remove(&a);
                }
            },
            DecodedOp::LoadK { a, bx } => {
                registers.insert(a, Known::Const(constant(bx)?));
            }
            DecodedOp::LoadBool { a, b, .. } => {
                registers.insert(a, Known::Const(Constant::Bool(b != 0)));
            }
            DecodedOp::LoadNil { a, b } => {
                for register in a..=b {
                    registers.remove(&register);
                }
            }
            DecodedOp::GetGlobal { a, bx } => match constant(bx)? {
                Constant::Str(name) => {
                    registers.insert(a, Known::Global(name));
                }
                _ => {
                    registers.remove(&a);
                }
            },
            DecodedOp::GetTable { a, b, c } => {
                let value = match (registers.get(&b), rk(&registers, c)?) {
                    (Some(Known::Table(id)), Some(Known::Const(Constant::Str(key)))) => {
                        table_fields[*id]
                            .iter()
                            .rev()
                            .find(|(name, _)| *name == key)
                            .map(|(_, value)| value.clone())
                    }
                    _ => None,
                };
                match value {
                    Some(value) => registers.insert(a, value),
                    None => registers.remove(&a),
                };
            }
            DecodedOp::SetGlobal { a, bx } => {
                if let (Some(Known::Table(id)), Constant::Str(name)) =
                    (registers.get(&a), constant(bx)?)
                {
                    globals.retain(|(existing, _)| *existing != name);
                    globals.push((name, *id));
                }
            }
            DecodedOp::SetTable { a, b, c } => {
                if let (
                    Some(Known::Table(id)),
                    Some(Known::Const(Constant::Str(key))),
                    Some(value),
                ) = (
                    registers.get(&a).cloned(),
                    rk(&registers, b)?,
                    rk(&registers, c)?,
                ) {
                    table_fields[id].retain(|(name, _)| *name != key);
                    if !matches!(value, Known::Const(Constant::Nil)) {
                        table_fields[id].push((key, value));
                    }
                }
            }
            DecodedOp::NewTable { a } => {
                if table_fields.len() >= MAX_ITEMS {
                    return Err("Too many Lua tables".into());
                }
                table_fields.push(Vec::new());
                table_items.push(Vec::new());
                registers.insert(a, Known::Table(table_fields.len() - 1));
            }
            DecodedOp::SelfOp { a } => {
                registers.remove(&a);
                registers.remove(&(a + 1));
            }
            DecodedOp::SetList { a, b, c } => {
                let block = if c == 0 {
                    let next = *code.get(pc).ok_or("Truncated SETLIST")? as usize;
                    pc += 1;
                    next
                } else {
                    c
                };
                if let Some(Known::Table(id)) = registers.get(&a).cloned()
                    && b != 0
                    && block != 0
                {
                    let first = (block - 1) * LFIELDS_PER_FLUSH;
                    // Only a contiguous, fully known list is recorded.
                    if table_items[id].len() == first {
                        let values: Option<Vec<_>> =
                            (1..=b).map(|n| registers.get(&(a + n)).cloned()).collect();
                        if let Some(values) = values {
                            table_items[id].extend(values);
                        }
                    }
                }
                registers.retain(|register, _| *register <= a);
            }
            DecodedOp::Closure { a, bx } => {
                registers.remove(&a);
                // The prototype's upvalues follow as pseudo-instructions.
                let upvalues = *main
                    .upvalue_counts
                    .get(bx)
                    .ok_or("Lua closure index out of range")?;
                pc += upvalues;
            }
            DecodedOp::Jump { .. } | DecodedOp::SkipNextClear | DecodedOp::Clear => {
                registers.clear();
            }
            DecodedOp::Invalidate(a) => {
                registers.remove(&a);
            }
            DecodedOp::SkipNext | DecodedOp::Noop => {}
        }
    }
    let mut out = Vec::new();
    for (name, id) in globals {
        let mut visiting = Vec::new();
        out.push((
            name,
            materialize(id, &table_fields, &table_items, &mut visiting, 0),
        ));
    }
    Ok(out)
}

fn materialize(
    id: usize,
    fields: &[Vec<(String, Known)>],
    items: &[Vec<Known>],
    visiting: &mut Vec<usize>,
    depth: usize,
) -> Table {
    if depth > MAX_TABLE_DEPTH || visiting.contains(&id) {
        return Table::default();
    }
    visiting.push(id);
    let convert = |value: &Known, visiting: &mut Vec<usize>| -> Option<Value> {
        Some(match value {
            Known::Const(Constant::Str(text)) => Value::Str(text.clone()),
            Known::Const(Constant::Num(number)) => Value::Num(*number),
            Known::Const(Constant::Bool(flag)) => Value::Bool(*flag),
            Known::Const(Constant::Nil) => return None,
            Known::Global(name) => Value::Global(name.clone()),
            Known::Table(child) => {
                Value::Table(materialize(*child, fields, items, visiting, depth + 1))
            }
        })
    };
    let table = Table {
        fields: fields[id]
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), convert(value, visiting)?)))
            .collect(),
        items: items[id]
            .iter()
            .filter_map(|value| convert(value, visiting))
            .collect(),
    };
    visiting.pop();
    table
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[allow(dead_code)]
    pub mod lua_op {
        pub const MOVE: u32 = 0;
        pub const LOADK: u32 = 1;
        pub const LOADBOOL: u32 = 2;
        pub const LOADNIL: u32 = 3;
        pub const GETGLOBAL: u32 = 5;
        pub const GETTABLE: u32 = 6;
        pub const SETGLOBAL: u32 = 7;
        pub const SETTABLE: u32 = 9;
        pub const NEWTABLE: u32 = 10;
        pub const SELF: u32 = 11;
        pub const JMP: u32 = 22;
        pub const CALL: u32 = 28;
        pub const RETURN: u32 = 30;
        pub const SETLIST: u32 = 34;
        pub const CLOSURE: u32 = 36;
    }

    #[allow(dead_code)]
    pub mod jee_op {
        pub const GETTABLE: u32 = 2;
        pub const LOADBOOL: u32 = 3;
        pub const SETTABLE: u32 = 7;
        pub const CALL: u32 = 8;
        pub const FORPREP: u32 = 10;
        pub const LOADK: u32 = 11;
        pub const SETGLOBAL: u32 = 12;
        pub const MOVE: u32 = 13;
        pub const LOADNIL: u32 = 16;
        pub const TEST: u32 = 17;
        pub const NEWTABLE: u32 = 19;
        pub const JMP: u32 = 21;
        pub const RETURN: u32 = 23;
        pub const FORLOOP: u32 = 27;
        pub const GETGLOBAL: u32 = 28;
        pub const CLOSURE: u32 = 29;
        pub const SELF: u32 = 30;
        pub const SETLIST: u32 = 33;
    }

    use lua_op::*;

    pub enum K<'a> {
        S(&'a str),
        N(f64),
    }

    pub fn abc(op: u32, a: u32, b: u32, c: u32) -> u32 {
        op | (a << 6) | (c << 14) | (b << 23)
    }
    pub fn abx(op: u32, a: u32, bx: u32) -> u32 {
        op | (a << 6) | (bx << 14)
    }
    pub fn asbx(op: u32, a: u32, sbx: i32) -> u32 {
        abx(op, a, (sbx + 131_071) as u32)
    }

    fn string(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&((text.len() + 1) as u64).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }

    fn function(out: &mut Vec<u8>, code: &[u32], constants: &[K], nested: &[(u8, &[u32])]) {
        string(out, "@test.lua");
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&[0, 0, 2, 20]);
        out.extend_from_slice(&(code.len() as u32).to_le_bytes());
        for word in code {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&(constants.len() as u32).to_le_bytes());
        for constant in constants {
            match constant {
                K::S(text) => {
                    out.push(4);
                    string(out, text);
                }
                K::N(number) => {
                    out.push(3);
                    out.extend_from_slice(&(*number as f32).to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&(nested.len() as u32).to_le_bytes());
        for (upvalues, code) in nested {
            string(out, "@test.lua");
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&[*upvalues, 0, 2, 2]);
            out.extend_from_slice(&(code.len() as u32).to_le_bytes());
            for word in *code {
                out.extend_from_slice(&word.to_le_bytes());
            }
            out.extend_from_slice(&[0; 20]);
        }
        out.extend_from_slice(&[0; 12]);
    }

    /// A Legacy-shaped chunk: 1-byte-aligned header with 8-byte size_t and
    /// 4-byte float numbers, as in the shipped scripts.
    pub fn chunk(code: &[u32], constants: &[K], nested: &[(u8, &[u32])]) -> Vec<u8> {
        let mut out = b"\x1bLuaQ\x00\x01\x04\x08\x04\x04\x00".to_vec();
        function(&mut out, code, constants, nested);
        out
    }

    fn jee_function(out: &mut Vec<u8>, code: &[u32], constants: &[K], nested: &[(u8, &[u32])]) {
        string(out, "@test.lua");
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&[0, 0, 2, 20]);
        out.extend_from_slice(&(constants.len() as u32).to_le_bytes());
        for constant in constants {
            match constant {
                K::S(text) => {
                    out.push(4);
                    string(out, text);
                }
                K::N(number) => {
                    out.push(3);
                    out.extend_from_slice(&(*number as f32).to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&(nested.len() as u32).to_le_bytes());
        for (upvalues, child_code) in nested {
            string(out, "@test.lua");
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&[*upvalues, 0, 2, 2]);
            out.extend_from_slice(&[0; 8]); // 0 constants, 0 children
            out.extend_from_slice(&(child_code.len() as u32).to_le_bytes());
            for word in *child_code {
                out.extend_from_slice(&word.to_le_bytes());
            }
            out.extend_from_slice(&[0; 12]); // 0 lineinfo, 0 locals, 0 upvalues
        }
        out.extend_from_slice(&(code.len() as u32).to_le_bytes());
        for word in code {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&[0; 12]); // 0 lineinfo, 0 locals, 0 upvalues
    }

    /// A Stage 2 Jee chunk.
    pub fn jee_chunk(code: &[u32], constants: &[K], nested: &[(u8, &[u32])]) -> Vec<u8> {
        let mut out = b"\x1bJee".to_vec();
        jee_function(&mut out, code, constants, nested);
        out
    }

    #[test]
    fn reads_literal_subtype_tables_without_running_code() {
        // GoliathMeteor1 = { baseType = GoliathA1, Properties = { fileModel = "m.cdf",
        //   health = 9 }, loadout = { "Charge1", "Breath1" } }
        let constants = [
            K::S("GoliathA1"),
            K::S("baseType"),
            K::S("Properties"),
            K::S("fileModel"),
            K::S("m.cdf"),
            K::S("health"),
            K::N(9.0),
            K::S("loadout"),
            K::S("Charge1"),
            K::S("Breath1"),
            K::S("GoliathMeteor1"),
        ];
        let code = [
            abc(NEWTABLE, 0, 0, 0),
            abx(GETGLOBAL, 1, 0),
            abc(SETTABLE, 0, 256 + 1, 1),
            abx(LOADK, 1, 2),
            abc(NEWTABLE, 2, 0, 0),
            abx(LOADK, 3, 3),
            abc(SETTABLE, 2, 3, 256 + 4),
            abc(SETTABLE, 2, 256 + 5, 256 + 6),
            abc(SETTABLE, 0, 1, 2),
            abc(NEWTABLE, 1, 2, 0),
            abx(LOADK, 2, 8),
            abx(LOADK, 3, 9),
            abc(SETLIST, 1, 2, 1),
            abc(SETTABLE, 0, 256 + 7, 1),
            abx(SETGLOBAL, 0, 10),
            abc(30, 0, 1, 0),
        ];
        let globals = authored_globals(&chunk(&code, &constants, &[])).unwrap();
        assert_eq!(globals.len(), 1);
        let (name, table) = &globals[0];
        assert_eq!(name, "GoliathMeteor1");
        assert_eq!(
            table.get("baseType"),
            Some(&Value::Global("GoliathA1".into()))
        );
        let properties = table.table("Properties").unwrap();
        assert_eq!(properties.str("fileModel"), Some("m.cdf"));
        assert_eq!(properties.get("health"), Some(&Value::Num(9.0)));
        assert_eq!(
            table.table("loadout").unwrap().items,
            [Value::Str("Charge1".into()), Value::Str("Breath1".into())]
        );
        let mut flat = Vec::new();
        table.flatten("GoliathMeteor1", &mut flat);
        assert!(flat.contains(&(
            "GoliathMeteor1 / Properties / fileModel".into(),
            "m.cdf".into()
        )));
    }

    #[test]
    fn reads_literal_subtype_tables_from_jee_bytecode() {
        let constants = [
            K::S("GoliathBase"),
            K::S("baseType"),
            K::S("Properties"),
            K::S("fileModel"),
            K::S("m.cdf"),
            K::S("health"),
            K::N(9.0),
            K::S("loadout"),
            K::S("Charge1"),
            K::S("Breath1"),
            K::S("Goliath1"),
        ];
        let code = [
            abc(jee_op::NEWTABLE, 0, 0, 0),
            abx(jee_op::GETGLOBAL, 1, 0),
            abc(jee_op::SETTABLE, 0, 256 + 1, 1),
            abx(jee_op::LOADK, 1, 2),
            abc(jee_op::NEWTABLE, 2, 0, 0),
            abx(jee_op::LOADK, 3, 3),
            abc(jee_op::SETTABLE, 2, 3, 256 + 4),
            abc(jee_op::SETTABLE, 2, 256 + 5, 256 + 6),
            abc(jee_op::SETTABLE, 0, 1, 2),
            abc(jee_op::NEWTABLE, 1, 2, 0),
            abx(jee_op::LOADK, 2, 8),
            abx(jee_op::LOADK, 3, 9),
            abc(jee_op::SETLIST, 1, 2, 1),
            abc(jee_op::SETTABLE, 0, 256 + 7, 1),
            abx(jee_op::SETGLOBAL, 0, 10),
            abc(jee_op::RETURN, 0, 1, 0),
        ];
        let globals = authored_globals(&jee_chunk(&code, &constants, &[])).unwrap();
        assert_eq!(globals.len(), 1);
        let (name, table) = &globals[0];
        assert_eq!(name, "Goliath1");
        assert_eq!(
            table.get("baseType"),
            Some(&Value::Global("GoliathBase".into()))
        );
        let properties = table.table("Properties").unwrap();
        assert_eq!(properties.str("fileModel"), Some("m.cdf"));
        assert_eq!(properties.get("health"), Some(&Value::Num(9.0)));
        assert_eq!(
            table.table("loadout").unwrap().items,
            [Value::Str("Charge1".into()), Value::Str("Breath1".into())]
        );
        let mut flat = Vec::new();
        table.flatten("Goliath1", &mut flat);
        assert!(flat.contains(&("Goliath1 / Properties / fileModel".into(), "m.cdf".into())));
    }

    #[test]
    fn branches_calls_and_closures_forget_registers() {
        let constants = [K::S("fileModel"), K::S("a.cdf"), K::S("T"), K::S("b.cdf")];
        let code = [
            abc(NEWTABLE, 0, 0, 0),
            abx(LOADK, 1, 0),
            // A jump lands on the next SETTABLE, so its key register is unknown.
            asbx(JMP, 0, 0),
            abc(SETTABLE, 0, 1, 256 + 1),
            // A call clobbers the table register itself.
            abx(LOADK, 1, 0),
            abc(28, 0, 1, 1),
            abc(SETTABLE, 0, 1, 256 + 3),
            abc(NEWTABLE, 0, 0, 0),
            // CLOSURE with one upvalue: the following MOVE is a pseudo-op and
            // must not clear the table in register 0.
            abx(CLOSURE, 2, 0),
            abc(0, 0, 3, 0),
            abx(LOADK, 1, 0),
            abc(SETTABLE, 0, 1, 256 + 1),
            abx(SETGLOBAL, 0, 2),
            abc(30, 0, 1, 0),
        ];
        let globals =
            authored_globals(&chunk(&code, &constants, &[(1, &[abc(30, 0, 1, 0)])])).unwrap();
        assert_eq!(globals.len(), 1);
        assert_eq!(globals[0].1.fields.len(), 1);
        assert_eq!(
            globals[0].1.get("fileModel"),
            Some(&Value::Str("a.cdf".into()))
        );
    }

    #[test]
    fn jee_branches_calls_and_closures_forget_registers() {
        let constants = [K::S("fileModel"), K::S("a.cdf"), K::S("T"), K::S("b.cdf")];
        let code = [
            abc(jee_op::NEWTABLE, 0, 0, 0),
            abx(jee_op::LOADK, 1, 0),
            // A jump lands on the next SETTABLE, so its key register is unknown.
            asbx(jee_op::JMP, 0, 0),
            abc(jee_op::SETTABLE, 0, 1, 256 + 1),
            // A call clobbers the table register itself.
            abx(jee_op::LOADK, 1, 0),
            abc(jee_op::CALL, 0, 1, 1),
            abc(jee_op::SETTABLE, 0, 1, 256 + 3),
            abc(jee_op::NEWTABLE, 0, 0, 0),
            // CLOSURE with one upvalue: the following MOVE is a pseudo-op and
            // must not clear the table in register 0.
            abx(jee_op::CLOSURE, 2, 0),
            abc(jee_op::MOVE, 0, 3, 0),
            abx(jee_op::LOADK, 1, 0),
            abc(jee_op::SETTABLE, 0, 1, 256 + 1),
            abx(jee_op::SETGLOBAL, 0, 2),
            abc(jee_op::RETURN, 0, 1, 0),
        ];
        let globals = authored_globals(&jee_chunk(
            &code,
            &constants,
            &[(1, &[abc(jee_op::RETURN, 0, 1, 0)])],
        ))
        .unwrap();
        assert_eq!(globals.len(), 1);
        assert_eq!(globals[0].1.fields.len(), 1);
        assert_eq!(
            globals[0].1.get("fileModel"),
            Some(&Value::Str("a.cdf".into()))
        );
    }

    #[test]
    fn rejects_foreign_truncated_or_oversized_bytecode() {
        let good = chunk(&[abc(30, 0, 1, 0)], &[], &[]);
        assert!(authored_globals(&good).unwrap().is_empty());
        assert!(authored_globals(&good[..good.len() - 1]).is_err());
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(authored_globals(&trailing).is_err());
        let mut big_endian = good.clone();
        big_endian[6] = 0;
        assert!(authored_globals(&big_endian).is_err());
        assert!(authored_globals(b"\x1bJee\0\0").is_err());
        assert!(authored_globals(b"fileModel = \"x.cdf\"").is_err());
        let mut huge_count = good;
        // The code count of the main function: claims more words than exist.
        let at = 12 + 8 + 10 + 8 + 4;
        huge_count[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(authored_globals(&huge_count).is_err());

        // Stage 2 Jee checks
        let good_jee = jee_chunk(&[abc(jee_op::RETURN, 0, 1, 0)], &[], &[]);
        assert!(authored_globals(&good_jee).unwrap().is_empty());
        assert!(authored_globals(&good_jee[..good_jee.len() - 1]).is_err());
        let mut trailing_jee = good_jee.clone();
        trailing_jee.push(0);
        assert!(authored_globals(&trailing_jee).is_err());
        let mut huge_count_jee = good_jee;
        // The code count of the main function: claims more words than exist.
        // Jee header: 4 (magic) + 8 (source len) + 10 (source "@test.lua\0") + 8 (lines) + 4 (flags) + 4 (constants count = 0) + 4 (nested count = 0) = 42
        let at_jee = 4 + 8 + 10 + 8 + 4 + 4 + 4;
        huge_count_jee[at_jee..at_jee + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(authored_globals(&huge_count_jee).is_err());
    }
}
