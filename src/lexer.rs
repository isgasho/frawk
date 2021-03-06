//! A custom lexer for AWK for use with LALRPOP.
//!
//! This lexer is fairly rudamentary. It ought not be too slow, but it also has not been optimized
//! very aggressively. Various edge cases still do not work.
use hashbrown::HashMap;
use regex::Regex;
use unicode_xid::UnicodeXID;

use crate::arena::Arena;

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Loc {
    pub line: usize,
    pub col: usize,
    offset: usize,
}

pub type Spanned<T> = (Loc, T, Loc);

#[derive(Debug, PartialEq, Clone)]
pub enum Tok<'a> {
    Begin,
    Prepare,
    End,
    Break,
    Continue,
    Next,
    NextFile,
    For,
    If,
    Else,
    Print,
    Printf,
    // Separate token for a "print(" and "printf(".
    PrintLP,
    PrintfLP,
    While,
    Do,

    // { }
    LBrace,
    RBrace,
    // [ ]
    LBrack,
    RBrack,
    // ( )
    LParen,
    RParen,

    Getline,
    Assign,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Mul,
    MulAssign,
    Div,
    DivAssign,
    Pow,
    PowAssign,
    Mod,
    ModAssign,
    Match,
    NotMatch,

    EQ,
    NEQ,
    LT,
    GT,
    LTE,
    GTE,
    Incr,
    Decr,
    Not,

    AND,
    OR,
    QUESTION,
    COLON,

    Append, // >>

    Dollar,
    Semi,
    Newline,
    Comma,
    In,
    Delete,
    Function,
    Return,

    Ident(&'a str),
    StrLit(&'a str),
    PatLit(&'a str),
    CallStart(&'a str),

    ILit(&'a str),
    HexLit(&'a str),
    FLit(&'a str),
}

static_map!(
    KEYWORDS<&'static str, Tok<'static>>,
    ["PREPARE", Tok::Prepare],
    ["BEGIN", Tok::Begin],
    ["END", Tok::End],
    ["break", Tok::Break],
    ["continue", Tok::Continue],
    ["next", Tok::Next],
    ["nextfile", Tok::NextFile],
    ["for", Tok::For],
    ["if", Tok::If],
    ["else", Tok::Else],
    ["print", Tok::Print],
    ["printf", Tok::Printf],
    ["print(", Tok::PrintLP],
    ["printf(", Tok::PrintfLP],
    ["while", Tok::While],
    ["do", Tok::Do],
    ["{", Tok::LBrace],
    ["}", Tok::RBrace],
    ["[", Tok::LBrack],
    ["]", Tok::RBrack],
    ["(", Tok::LParen],
    [")", Tok::RParen],
    ["getline", Tok::Getline],
    ["=", Tok::Assign],
    ["+", Tok::Add],
    ["+=", Tok::AddAssign],
    ["-", Tok::Sub],
    ["-=", Tok::SubAssign],
    ["*", Tok::Mul],
    ["*=", Tok::MulAssign],
    ["/", Tok::Div],
    ["/=", Tok::DivAssign],
    ["^", Tok::Pow],
    ["^=", Tok::PowAssign],
    ["%", Tok::Mod],
    ["%=", Tok::ModAssign],
    ["~", Tok::Match],
    ["!~", Tok::NotMatch],
    ["==", Tok::EQ],
    ["!=", Tok::NEQ],
    ["<", Tok::LT],
    ["<=", Tok::LTE],
    [">", Tok::GT],
    ["--", Tok::Decr],
    ["++", Tok::Incr],
    [">=", Tok::GTE],
    [">>", Tok::Append],
    [";", Tok::Semi],
    ["\n", Tok::Newline],
    ["\r\n", Tok::Newline],
    [",", Tok::Comma],
    // XXX: hack "in" must have whitespace after it.
    ["in ", Tok::In],
    ["in\t", Tok::In],
    ["!", Tok::Not],
    ["&&", Tok::AND],
    ["||", Tok::OR],
    ["?", Tok::QUESTION],
    [":", Tok::COLON],
    ["delete", Tok::Delete],
    ["function", Tok::Function],
    ["return", Tok::Return],
    ["$", Tok::Dollar]
);

use lazy_static::lazy_static;

lazy_static! {
    static ref KEYWORDS_BY_LEN: Vec<HashMap<&'static [u8], Tok<'static>>> = {
        let max_len = KEYWORDS.keys().map(|s| s.len()).max().unwrap();
        let mut res: Vec<HashMap<_, _>> = vec![Default::default(); max_len];
        for (k, v) in KEYWORDS.iter() {
            res[k.len() - 1].insert(k.as_bytes(), v.clone());
        }
        res
    };
}

pub struct Tokenizer<'a> {
    text: &'a str,
    cur: usize,
    prev_tok: Option<Tok<'a>>,
    lines: Vec<usize>,
}

fn is_id_start(c: char) -> bool {
    c == '_' || c.is_xid_start()
}

fn is_id_body(c: char) -> bool {
    c == '_' || c == '\'' || c.is_xid_continue()
}

fn push_char(buf: &mut Vec<u8>, c: char) {
    let start = buf.len();
    buf.resize_with(start + c.len_utf8(), Default::default);
    c.encode_utf8(&mut buf[start..]);
}

pub(crate) fn parse_string_literal<'a, 'outer>(
    lit: &str,
    arena: &'a Arena<'outer>,
    buf: &mut Vec<u8>,
) -> &'a str {
    // assumes we just saw a '"'
    buf.clear();
    let mut is_escape = false;
    for c in lit.chars() {
        if is_escape {
            match c {
                'a' => buf.push(0x07), // BEL
                'b' => buf.push(0x08), // BS
                'f' => buf.push(0x0C), // FF
                'v' => buf.push(0x0B), // VT
                '\\' => buf.push(b'\\'),
                'n' => buf.push(b'\n'),
                'r' => buf.push(b'\r'),
                't' => buf.push(b'\t'),
                '"' => buf.push(b'"'),
                c => {
                    buf.push(b'\\');
                    push_char(buf, c);
                }
            };
            is_escape = false;
        } else {
            match c {
                '\\' => {
                    is_escape = true;
                    continue;
                }
                c => {
                    push_char(buf, c);
                }
            }
        }
    }
    std::str::from_utf8(arena.alloc_bytes(&buf[..])).unwrap()
}

pub(crate) fn parse_regex_literal<'a, 'outer>(
    lit: &str,
    arena: &'a Arena<'outer>,
    buf: &mut Vec<u8>,
) -> &'a str {
    buf.clear();
    let mut is_escape = false;
    for c in lit.chars() {
        if is_escape {
            match c {
                '/' => buf.push(b'/'),
                c => {
                    buf.push(b'\\');
                    push_char(buf, c);
                }
            };
            is_escape = false;
        } else {
            match c {
                '\\' => {
                    is_escape = true;
                    continue;
                }
                '/' => {
                    break;
                }
                c => {
                    push_char(buf, c);
                }
            }
        }
    }
    std::str::from_utf8(arena.alloc_bytes(&buf[..])).unwrap()
}

impl<'a> Tokenizer<'a> {
    fn keyword<'c>(&self) -> Option<(Tok<'c>, usize)> {
        let start = self.cur;
        let remaining = self.text.len() - start;
        for (len, ks) in KEYWORDS_BY_LEN.iter().enumerate().rev() {
            let len = len + 1;
            if remaining < len {
                continue;
            }
            if let Some(tok) = ks.get(&self.text.as_bytes()[start..start + len]) {
                return Some((tok.clone(), len));
            }
        }
        None
    }

    fn num(&self) -> Option<(Tok<'a>, usize)> {
        lazy_static! {
            static ref HEX_PATTERN: Regex = Regex::new(r"^[+-]?0[xX][0-9A-Fa-f]+").unwrap();
            static ref INT_PATTERN: Regex = Regex::new(r"^[+-]?\d+").unwrap();
            // Adapted from https://www.regular-expressions.info/floatingpoint.html
            static ref FLOAT_PATTERN: Regex = Regex::new(r"^[-+]?\d*\.\d+([eE][-+]?\d+)?").unwrap();
        };
        let text = &self.text[self.cur..];
        if let Some(i) = HEX_PATTERN.captures(text).and_then(|c| c.get(0)) {
            let is = i.as_str();
            return Some((Tok::HexLit(is), is.len()));
        } else if let Some(f) = FLOAT_PATTERN.captures(text).and_then(|c| c.get(0)) {
            let fs = f.as_str();
            Some((Tok::FLit(fs), fs.len()))
        } else if let Some(i) = INT_PATTERN.captures(text).and_then(|c| c.get(0)) {
            let is = i.as_str();
            Some((Tok::ILit(is), is.len()))
        } else {
            None
        }
    }

    fn ident(&mut self, id_start: usize) -> (&'a str, usize) {
        debug_assert!(is_id_start(self.text[id_start..].chars().next().unwrap()));
        let ix = self.text[self.cur..]
            .char_indices()
            .take_while(|(_, c)| is_id_body(*c))
            .last()
            .map(|(ix, _)| self.cur + ix + 1)
            .unwrap_or(self.cur);
        (&self.text[id_start..ix], ix)
    }

    fn literal(&mut self, delim: char, error_msg: &'static str) -> Result<(&'a str, usize), Error> {
        // assumes we just saw a delimiter.
        let mut bound = None;
        let mut is_escape = false;
        for (ix, c) in self.text[self.cur..].char_indices() {
            if is_escape {
                is_escape = false;
                continue;
            }
            if c == delim {
                bound = Some(ix);
                break;
            }
            if c == '\\' {
                is_escape = true;
            }
        }
        match bound {
            Some(end) => Ok((&self.text[self.cur..self.cur + end], self.cur + end + 1)),
            None => Err(Error {
                location: self.index_to_loc(self.cur),
                desc: error_msg,
            }),
        }
    }

    fn regex_lit(&mut self) -> Result<(&'a str, usize /* new start */), Error> {
        self.literal('/', "incomplete regex literal")
    }

    fn string_lit(&mut self) -> Result<(&'a str, usize /* new start */), Error> {
        self.literal('"', "incomplete string literal")
    }

    fn consume_comment(&mut self) {
        let mut iter = self.text[self.cur..].char_indices();
        if let Some((_, '#')) = iter.next() {
            if let Some((ix, _)) = iter.skip_while(|x| x.1 != '\n').next() {
                self.cur += ix;
            } else {
                self.cur = self.text.len();
            }
        }
    }

    fn consume_ws(&mut self) {
        let mut res = 0;
        for (ix, c) in self.text[self.cur..].char_indices() {
            res = ix;
            if c == '\n' || !c.is_whitespace() {
                break;
            }
        }
        self.cur += res;
    }

    fn advance(&mut self) {
        let mut prev = self.cur;
        loop {
            self.consume_ws();
            self.consume_comment();
            if self.cur == prev {
                break;
            }
            prev = self.cur;
        }
    }

    fn potential_re(&self) -> bool {
        match &self.prev_tok {
            Some(Tok::Ident(_)) | Some(Tok::StrLit(_)) | Some(Tok::PatLit(_))
            | Some(Tok::ILit(_)) | Some(Tok::FLit(_)) | Some(Tok::RParen) => false,
            _ => true,
        }
    }
}

#[derive(Debug)]
pub struct Error {
    pub location: Loc,
    pub desc: &'static str,
}

impl<'a> Tokenizer<'a> {
    pub fn new(text: &'a str) -> Tokenizer<'a> {
        Tokenizer {
            text,
            cur: 0,
            prev_tok: None,
            lines: text
                .as_bytes()
                .iter()
                .enumerate()
                .flat_map(|(i, b)| if *b == b'\n' { Some(i) } else { None }.into_iter())
                .collect(),
        }
    }
    fn index_to_loc(&self, ix: usize) -> Loc {
        let offset = ix;
        match self.lines.binary_search(&ix) {
            Ok(0) | Err(0) => Loc {
                line: 0,
                col: ix,
                offset,
            },
            Ok(line) => Loc {
                line: line - 1,
                col: ix - self.lines[line - 1] - 1,
                offset,
            },
            Err(line) => Loc {
                line,
                col: ix - self.lines[line - 1] - 1,
                offset,
            },
        }
    }
    fn spanned<T>(&self, l: usize, r: usize, t: T) -> Spanned<T> {
        (self.index_to_loc(l), t, self.index_to_loc(r))
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Result<Spanned<Tok<'a>>, Error>;
    fn next(&mut self) -> Option<Result<Spanned<Tok<'a>>, Error>> {
        macro_rules! try_tok {
            ($e:expr) => {
                match $e {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
            };
        }
        self.advance();
        let span = if let Some((ix, c)) = self.text[self.cur..].char_indices().next() {
            let ix = self.cur + ix;
            match c {
                '"' => {
                    self.cur += 1;
                    let (s, new_start) = try_tok!(self.string_lit());
                    self.cur = new_start;
                    self.spanned(ix, new_start, Tok::StrLit(s))
                }
                '/' if self.potential_re() => {
                    self.cur += 1;
                    let (re, new_start) = try_tok!(self.regex_lit());
                    self.cur = new_start;
                    self.spanned(ix, new_start, Tok::PatLit(re))
                }
                c => {
                    if let Some((tok, len)) = self.keyword() {
                        self.cur += len;
                        self.spanned(ix, self.cur, tok)
                    } else if let Some((tok, len)) = self.num() {
                        self.cur += len;
                        self.spanned(ix, self.cur, tok)
                    } else if is_id_start(c) {
                        self.cur += c.len_utf8();
                        let (s, new_start) = self.ident(ix);
                        let bs = self.text.as_bytes();
                        if new_start < bs.len() && self.text.as_bytes()[new_start] == b'(' {
                            self.cur = new_start + 1;
                            self.spanned(ix, self.cur, Tok::CallStart(s))
                        } else {
                            self.cur = new_start;
                            self.spanned(ix, self.cur, Tok::Ident(s))
                        }
                    } else {
                        return None;
                    }
                }
            }
        } else {
            return None;
        };
        self.prev_tok = Some(span.1.clone());
        Some(Ok(span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lex_str<'b>(s: &'b str) -> Vec<Spanned<Tok<'b>>> {
        Tokenizer::new(s).map(|x| x.ok().unwrap()).collect()
    }

    #[test]
    fn locations() {
        const TEXT: &'static str = r#"This is the first line
and the second
and the third"#;
        let tok = Tokenizer::new(TEXT);
        assert_eq!(
            tok.index_to_loc(4),
            Loc {
                line: 0,
                col: 4,
                offset: 4,
            }
        );
        assert_eq!(
            tok.index_to_loc(22),
            Loc {
                line: 0,
                col: 22,
                offset: 22,
            }
        );
        assert_eq!(
            tok.index_to_loc(23),
            Loc {
                line: 1,
                col: 0,
                offset: 23,
            }
        );
        let tok2 = Tokenizer::new("\nhello");
        assert_eq!(
            tok2.index_to_loc(0),
            Loc {
                line: 0,
                col: 0,
                offset: 0
            },
        );
        assert_eq!(
            tok2.index_to_loc(1),
            Loc {
                line: 1,
                col: 0,
                offset: 1
            },
        );
        assert_eq!(
            tok2.index_to_loc(2),
            Loc {
                line: 1,
                col: 1,
                offset: 2
            },
        );
    }

    #[test]
    fn basic() {
        let toks = lex_str(
            r#" if (x == yzk){
            print x<y, y<=z, z;
        }"#,
        );
        use Tok::*;
        assert_eq!(
            toks.into_iter().map(|x| x.1).collect::<Vec<_>>(),
            vec![
                If,
                LParen,
                Ident("x"),
                EQ,
                Ident("yzk"),
                RParen,
                LBrace,
                Newline,
                Print,
                Ident("x"),
                LT,
                Ident("y"),
                Comma,
                Ident("y"),
                LTE,
                Ident("z"),
                Comma,
                Ident("z"),
                Semi,
                Newline,
                RBrace
            ]
        );
    }

    #[test]
    fn literals() {
        let toks =
            lex_str(r#" x="\"hi\tthere\n"; b   =/hows it \/going/; x="重庆辣子鸡"; c= 1 / 3.5 "#);
        use Tok::*;
        let s1 = "\\\"hi\\tthere\\n";
        let s2 = "hows it \\/going";
        assert_eq!(
            toks.into_iter().map(|x| x.1).collect::<Vec<_>>(),
            vec![
                Ident("x"),
                Assign,
                StrLit(s1),
                Semi,
                Ident("b"),
                Assign,
                PatLit(s2),
                Semi,
                Ident("x"),
                Assign,
                StrLit("重庆辣子鸡"),
                Semi,
                Ident("c"),
                Assign,
                ILit("1"),
                Div,
                FLit("3.5"),
            ],
        );
        let mut buf = Vec::new();
        let a = Arena::default();
        assert_eq!(parse_string_literal(s1, &a, &mut buf), "\"hi\tthere\n");
        assert_eq!(parse_regex_literal(s2, &a, &mut buf), "hows it /going");
    }
}
