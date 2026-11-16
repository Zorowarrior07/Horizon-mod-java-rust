//! `hb-classfile`: what a mod's Java code does, read from `.class` files.
//!
//! No JVM, no decompiler, no bytecode execution. The bridge only needs to
//! answer questions that decide whether something can be ported:
//!
//! * Which loader entry points does the mod declare? (`@Mod`, `ModInitializer`,
//!   `ClientModInitializer`, `@EventBusSubscriber`, main-class manifest.)
//! * Does it use Mixins? (`@Mixin`, `*.mixins.json`, `@Redirect`, `@Inject`.)
//! * Which game classes does it extend — i.e. what *is* each class? A class
//!   extending `Block` with overrides is a block with custom behaviour: its
//!   texture and model port, its behaviour does not.
//! * Which registries does it touch? (`Registry.register`, `DeferredRegister`,
//!   `RegisterEvent`.)
//! * Which optional libraries does it need? (`com/simibubi/create`,
//!   `software/bernie/geckolib`, ...) — those become explicit "needs a separate
//!   reimplementation" lines rather than silent losses.
//!
//! The parser reads the class file structure (JVMS §4) strictly and reports
//! what it does not understand instead of guessing. Obfuscated names are
//! handled by reporting the package (`net/minecraft/...`) rather than
//! pretending to know which class it was.

use std::collections::{BTreeMap, BTreeSet};

/// A parsed class file, reduced to the facts the bridge uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClassFile {
    /// Binary name, `com/example/ModBlocks`.
    pub name: String,
    pub super_name: Option<String>,
    pub interfaces: Vec<String>,
    /// Declared methods: name, descriptor, access flags.
    pub methods: Vec<Method>,
    /// Declared fields: name, descriptor.
    pub fields: Vec<Field>,
    /// Every class named in the constant pool (`net/minecraft/...`).
    pub class_refs: BTreeSet<String>,
    /// Every method reference as `owner/name descriptor`.
    pub method_refs: Vec<MethodRef>,
    /// Class-level annotations by descriptor, e.g.
    /// `Lorg/spongepowered/asm/mixin/Mixin;`.
    pub annotations: BTreeSet<String>,
    /// Every annotation descriptor anywhere in the class (methods, fields,
    /// parameters).
    pub all_annotations: BTreeSet<String>,
    /// `SourceFile`, when present.
    pub source_file: Option<String>,
    /// String constants, used to spot file paths and registry names.
    pub strings: BTreeSet<String>,
    pub major_version: u16,
    /// True when the class is an interface or abstract.
    pub is_interface: bool,
    pub is_abstract: bool,
    /// Every `InnerClasses` entry's name.
    pub inner_classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Method {
    pub name: String,
    pub descriptor: String,
    pub access: u16,
    pub annotations: BTreeSet<String>,
}

impl Method {
    pub fn is_static(&self) -> bool {
        self.access & 0x0008 != 0
    }

    pub fn is_abstract(&self) -> bool {
        self.access & 0x0400 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub descriptor: String,
    pub access: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MethodRef {
    pub owner: String,
    pub name: String,
    pub descriptor: String,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

fn fail<T>(message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        message: message.into(),
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u1(&mut self) -> Result<u8, ParseError> {
        let b = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| ParseError { message: "truncated class file".into() })?;
        self.at += 1;
        Ok(b)
    }

    fn u2(&mut self) -> Result<u16, ParseError> {
        let hi = self.u1()? as u16;
        let lo = self.u1()? as u16;
        Ok((hi << 8) | lo)
    }

    fn u4(&mut self) -> Result<u32, ParseError> {
        let hi = self.u2()? as u32;
        let lo = self.u2()? as u32;
        Ok((hi << 16) | lo)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.bytes.len())
            .ok_or_else(|| ParseError { message: "truncated class file".into() })?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        self.take(n).map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Constant {
    Utf8(String),
    Class(u16),
    String(u16),
    /// Fieldref, Methodref, InterfaceMethodref, MethodHandle, etc.: the
    /// class and name-and-type indexes that matter.
    Ref { class: u16, name_type: u16 },
    NameAndType { name: u16, descriptor: u16 },
    /// Integer, Float, Long, Double, MethodType, Dynamic, InvokeDynamic,
    /// Module, Package: kept only for index bookkeeping.
    Other,
    Unusable,
}

/// Totals a jar's classes produce, for the report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Census {
    pub classes: usize,
    /// Classes under `net/minecraft/` that a mod class extends or implements.
    pub game_extends: BTreeSet<String>,
    pub mixin_classes: BTreeSet<String>,
    pub mixin_annotation_count: usize,
    pub entry_points: BTreeSet<String>,
    /// Registry classes the code refers to (`net/minecraft/core/registries/...`).
    pub registries: BTreeSet<String>,
    /// Non-game, non-loader third-party packages referenced.
    pub libraries: BTreeSet<String>,
    /// Behaviour hooks a content-only port cannot reproduce, one line each.
    pub behaviour: BTreeSet<String>,
    pub declares_classes_with_behaviour: usize,
}

/// Parses one class file.
pub fn parse(bytes: &[u8]) -> Result<ClassFile, ParseError> {
    let mut r = Reader { bytes, at: 0 };
    if r.u4()? != 0xCAFE_BABE {
        return fail("not a class file: bad magic (is this a Java 21+ class? those still start CAFEBABE)");
    }
    let minor = r.u2()?;
    let major = r.u2()?;
    if !(45..=70).contains(&major) {
        return fail(format!(
            "class file major version {major}.{minor} is outside 45-70 (Java 1.1 to 26)"
        ));
    }
    let pool_count = r.u2()? as usize;
    let mut pool: Vec<Constant> = Vec::with_capacity(pool_count);
    pool.push(Constant::Unusable); // index 0 is invalid
    let mut index = 1;
    while index < pool_count {
        let tag = r.u1()?;
        let constant = match tag {
            1 => {
                let len = r.u2()? as usize;
                let bytes = r.take(len)?;
                // Modified UTF-8. Decode the subset that matters; NUL is
                // encoded as C0 80 and supplementary characters as surrogate
                // pairs, both of which `String::from_utf8` refuses.
                Constant::Utf8(decode_modified_utf8(bytes))
            }
            3 | 4 => {
                r.skip(4)?;
                Constant::Other
            }
            5 | 6 => {
                r.skip(8)?;
                // Long and double take two pool slots.
                index += 1;
                Constant::Other
            }
            7 => Constant::Class(r.u2()?),
            8 => Constant::String(r.u2()?),
            9 | 10 | 11 => Constant::Ref {
                class: r.u2()?,
                name_type: r.u2()?,
            },
            12 => Constant::NameAndType {
                name: r.u2()?,
                descriptor: r.u2()?,
            },
            15 => {
                r.skip(3)?;
                Constant::Other
            }
            16 | 19 | 20 => {
                r.skip(2)?;
                Constant::Other
            }
            17 | 18 => {
                r.skip(4)?;
                Constant::Other
            }
            other => return fail(format!("unknown constant pool tag {other} at #{index}")),
        };
        pool.push(constant);
        index += 1;
    }

    let utf8 = |at: u16| -> Option<&str> {
        match pool.get(at as usize) {
            Some(Constant::Utf8(s)) => Some(s.as_str()),
            _ => None,
        }
    };
    let class_name = |at: u16| -> Option<String> {
        match pool.get(at as usize) {
            Some(Constant::Class(name_at)) => utf8(*name_at).map(|s| s.to_owned()),
            _ => None,
        }
    };

    let mut class = ClassFile {
        major_version: major,
        ..Default::default()
    };

    // Every class named in the pool, and every member reference.
    for constant in &pool {
        match constant {
            Constant::Class(name_at) => {
                if let Some(name) = utf8(*name_at) {
                    if is_classish(name) {
                        class.class_refs.insert(name.to_owned());
                    }
                }
            }
            Constant::String(s_at) => {
                if let Some(s) = utf8(*s_at) {
                    if s.len() <= 200 {
                        class.strings.insert(s.to_owned());
                    }
                }
            }
            Constant::Ref {
                class: class_at,
                name_type,
            } => {
                let owner = class_name(*class_at).unwrap_or_default();
                if let Some(Constant::NameAndType { name, descriptor }) = pool.get(*name_type as usize)
                {
                    let name = utf8(*name).unwrap_or_default();
                    let descriptor = utf8(*descriptor).unwrap_or_default();
                    class.method_refs.push(MethodRef {
                        owner: owner.clone(),
                        name: name.to_owned(),
                        descriptor: descriptor.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }

    let access = r.u2()?;
    class.is_interface = access & 0x0200 != 0;
    class.is_abstract = access & 0x0400 != 0;
    let this_class = r.u2()?;
    let super_class = r.u2()?;
    class.name = class_name(this_class).unwrap_or_default();
    if super_class != 0 {
        class.super_name = class_name(super_class);
    }
    let interface_count = r.u2()? as usize;
    for _ in 0..interface_count {
        let at = r.u2()?;
        if let Some(name) = class_name(at) {
            class.interfaces.push(name);
        }
    }

    // Fields.
    let field_count = r.u2()? as usize;
    for _ in 0..field_count {
        let field_access = r.u2()?;
        let name = utf8(r.u2()?).unwrap_or_default().to_owned();
        let descriptor = utf8(r.u2()?).unwrap_or_default().to_owned();
        class.fields.push(Field {
            name,
            descriptor,
            access: field_access,
        });
        skip_attributes(&mut r, &mut class, &utf8)?;
    }

    // Methods.
    let method_count = r.u2()? as usize;
    for _ in 0..method_count {
        let method_access = r.u2()?;
        let name = utf8(r.u2()?).unwrap_or_default().to_owned();
        let descriptor = utf8(r.u2()?).unwrap_or_default().to_owned();
        let mut annotations = BTreeSet::new();
        let attributes = r.u2()? as usize;
        for _ in 0..attributes {
            let attribute_name = utf8(r.u2()?).unwrap_or_default().to_owned();
            let length = r.u4()? as usize;
            let body = r.take(length)?;
            if attribute_name == "RuntimeVisibleAnnotations"
                || attribute_name == "RuntimeInvisibleAnnotations"
            {
                collect_annotations(body, &utf8, &mut annotations);
                for annotation in &annotations {
                    class.all_annotations.insert(annotation.clone());
                }
            }
        }
        for annotation in &annotations {
            class.all_annotations.insert(annotation.clone());
        }
        class.methods.push(Method {
            name,
            descriptor,
            access: method_access,
            annotations,
        });
    }

    // Class attributes: collect annotations and the source file name.
    let attribute_count = r.u2()? as usize;
    for _ in 0..attribute_count {
        let attribute_name = utf8(r.u2()?).unwrap_or_default().to_owned();
        let length = r.u4()? as usize;
        let body = r.take(length)?;
        match attribute_name.as_str() {
            "RuntimeVisibleAnnotations" | "RuntimeInvisibleAnnotations" => {
                let before = class.annotations.len();
                collect_annotations(body, &utf8, &mut class.annotations);
                let added = class.annotations.len() - before;
                let _ = added;
            }
            "SourceFile" => {
                let mut inner = Reader { bytes: body, at: 0 };
                if let Ok(at) = inner.u2() {
                    class.source_file = utf8(at).map(|s| s.to_owned());
                }
            }
            "InnerClasses" => {
                let mut inner = Reader { bytes: body, at: 0 };
                if let Ok(count) = inner.u2() {
                    for _ in 0..count {
                        let _inner_class = inner.u2();
                        let _outer = inner.u2();
                        let _name = inner.u2();
                        let _flags = inner.u2();
                        if inner.at > body.len() {
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for annotation in class.annotations.clone() {
        class.all_annotations.insert(annotation);
    }
    Ok(class)
}

fn skip_attributes<'a>(
    r: &mut Reader<'a>,
    class: &mut ClassFile,
    utf8: &impl Fn(u16) -> Option<&'a str>,
) -> Result<(), ParseError> {
    let count = r.u2()? as usize;
    for _ in 0..count {
        let name = utf8(r.u2()?).unwrap_or_default().to_owned();
        let length = r.u4()? as usize;
        let body = r.take(length)?;
        if name == "RuntimeVisibleAnnotations" || name == "RuntimeInvisibleAnnotations" {
            let mut found = BTreeSet::new();
            collect_annotations(body, utf8, &mut found);
            for annotation in found {
                class.all_annotations.insert(annotation);
            }
        }
    }
    Ok(())
}

/// Reads an `annotations` structure (JVMS 4.7.16) for annotation type
/// descriptors only; element values are skipped.
fn collect_annotations<'a>(
    body: &[u8],
    utf8: &impl Fn(u16) -> Option<&'a str>,
    out: &mut BTreeSet<String>,
) {
    let mut r = Reader { bytes: body, at: 0 };
    let Ok(count) = r.u2() else {
        return;
    };
    for _ in 0..count {
        if read_annotation(&mut r, utf8, out, 0).is_none() {
            // A malformed annotation table stops the scan; what was read so
            // far still stands, and the class remains usable.
            return;
        }
    }
}

fn read_annotation<'a>(
    r: &mut Reader<'_>,
    utf8: &impl Fn(u16) -> Option<&'a str>,
    out: &mut BTreeSet<String>,
    depth: u8,
) -> Option<()> {
    if depth > 32 {
        return None;
    }
    let type_at = r.u2().ok()?;
    if let Some(descriptor) = utf8(type_at) {
        out.insert(descriptor.to_owned());
    }
    let pairs = r.u2().ok()?;
    for _ in 0..pairs {
        r.u2().ok()?; // element name
        read_element_value(r, utf8, out, depth + 1)?;
    }
    Some(())
}

fn read_element_value<'a>(
    r: &mut Reader<'_>,
    utf8: &impl Fn(u16) -> Option<&'a str>,
    out: &mut BTreeSet<String>,
    depth: u8,
) -> Option<()> {
    if depth > 32 {
        return None;
    }
    match r.u1().ok()? {
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b'c' | b's' => {
            r.u2().ok()?;
            Some(())
        }
        b'e' => {
            r.u2().ok()?;
            r.u2().ok()?;
            Some(())
        }
        b'@' => read_annotation(r, utf8, out, depth + 1),
        b'[' => {
            let count = r.u2().ok()?;
            for _ in 0..count {
                read_element_value(r, utf8, out, depth + 1)?;
            }
            Some(())
        }
        _ => None,
    }
}

/// Modified UTF-8 (JVMS 4.4.7): NUL is two bytes, and supplementary
/// characters appear as encoded surrogate pairs.
fn decode_modified_utf8(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0 {
            // A literal NUL is not legal in modified UTF-8; keep going.
            out.push('\u{0}');
            i += 1;
        } else if b < 0x80 {
            out.push(b as char);
            i += 1;
        } else if b & 0xE0 == 0xC0 && i + 1 < bytes.len() {
            let cp = ((b as u32 & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F);
            i += 2;
            if (0xD800..0xDC00).contains(&cp) && i + 1 < bytes.len() && bytes[i] & 0xE0 == 0xC0 {
                // A surrogate pair encoded as two three-byte sequences.
                let low = ((bytes[i] as u32 & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F);
                if (0xDC00..0xE000).contains(&low) {
                    let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                    out.push(char::from_u32(combined).unwrap_or('\u{FFFD}'));
                    i += 2;
                    continue;
                }
            }
            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
        } else if b & 0xF0 == 0xE0 && i + 2 < bytes.len() {
            let cp = ((b as u32 & 0x0F) << 12)
                | ((bytes[i + 1] as u32 & 0x3F) << 6)
                | (bytes[i + 2] as u32 & 0x3F);
            i += 3;
            if (0xD800..0xDC00).contains(&cp) && i + 2 < bytes.len() && bytes[i] & 0xF0 == 0xE0 {
                let low = ((bytes[i] as u32 & 0x0F) << 12)
                    | ((bytes[i + 1] as u32 & 0x3F) << 6)
                    | (bytes[i + 2] as u32 & 0x3F);
                if (0xDC00..0xE000).contains(&low) {
                    let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                    out.push(char::from_u32(combined).unwrap_or('\u{FFFD}'));
                    i += 3;
                    continue;
                }
            }
            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
        } else {
            out.push('\u{FFFD}');
            i += 1;
        }
    }
    out
}

/// A constant-pool "class" entry may be an array descriptor (`[Lfoo/Bar;`);
/// only real names are interesting for our purposes.
fn is_classish(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('[') && name.contains('/')
}

/// Reads the `Main-Class` from a jar manifest.
pub fn main_class(manifest: &str) -> Option<String> {
    for line in manifest.lines() {
        if let Some(rest) = line.strip_prefix("Main-Class:") {
            return Some(rest.trim().replace('.', "/"));
        }
    }
    None
}

/// Annotations that mark a class as loader entry machinery.
const ENTRY_POINT_ANNOTATIONS: &[(&str, &str)] = &[
    (
        "Lnet/neoforged/fml/common/Mod;",
        "NeoForge @Mod class",
    ),
    ("Lnet/minecraftforge/fml/common/Mod;", "Forge @Mod class"),
    (
        "Lnet/neoforged/fml/common/EventBusSubscriber;",
        "NeoForge @EventBusSubscriber",
    ),
    (
        "Lnet/minecraftforge/fml/common/Mod/EventBusSubscriber;",
        "Forge @EventBusSubscriber",
    ),
];

const ENTRY_POINT_INTERFACES: &[(&str, &str)] = &[
    (
        "net/fabricmc/api/ModInitializer",
        "Fabric ModInitializer (common entry point)",
    ),
    (
        "net/fabricmc/api/ClientModInitializer",
        "Fabric ClientModInitializer (client entry point)",
    ),
    (
        "net/fabricmc/api/DedicatedServerModInitializer",
        "Fabric DedicatedServerModInitializer",
    ),
    (
        "net/neoforged/fml/common/Mod$",
        "NeoForge @Mod class",
    ),
    (
        "org/quiltmc/loader/api/ModInitializer",
        "Quilt ModInitializer",
    ),
];

/// Annotations that mark a Mixin member.
const MIXIN_MEMBER_ANNOTATIONS: &[(&str, &str)] = &[
    ("Lorg/spongepowered/asm/mixin/injection/Inject;", "@Inject"),
    ("Lorg/spongepowered/asm/mixin/injection/Redirect;", "@Redirect"),
    ("Lorg/spongepowered/asm/mixin/injection/ModifyArg;", "@ModifyArg"),
    ("Lorg/spongepowered/asm/mixin/injection/ModifyArgs;", "@ModifyArgs"),
    ("Lorg/spongepowered/asm/mixin/injection/ModifyVariable;", "@ModifyVariable"),
    ("Lorg/spongepowered/asm/mixin/injection/ModifyConstant;", "@ModifyConstant"),
    ("Lorg/spongepowered/asm/mixin/injection/At;", "@At"),
    ("Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;", "CallbackInfo"),
    ("Lorg/spongepowered/asm/mixin/Overwrite;", "@Overwrite"),
    ("Lorg/spongepowered/asm/mixin/gen/Accessor;", "@Accessor"),
    ("Lorg/spongepowered/asm/mixin/gen/Invoker;", "@Invoker"),
    ("Lorg/spongepowered/asm/mixin/Shadow;", "@Shadow"),
    ("Lorg/spongepowered/asm/mixin/Intrinsic;", "@Intrinsic"),
    ("Lorg/spongepowered/asm/mixin/Mixin;", "@Mixin"),
    ("Lcom/llamalad7/mixinextras/injector/ModifyExpressionValue;", "@ModifyExpressionValue (MixinExtras)"),
    ("Lcom/llamalad7/mixinextras/injector/WrapOperation;", "@WrapOperation (MixinExtras)"),
    ("Lorg/spongepowered/asm/mixin/injection/ModifyReceiver;", "@ModifyReceiver"),
];

/// Loaders' own packages: not third-party libraries, not game classes.
const LOADER_PREFIXES: &[&str] = &[
    "net/minecraftforge/",
    "net/neoforged/",
    "net/fabricmc/",
    "org/quiltmc/",
    "org/spongepowered/",
    "cpw/mods/",
    "com/llamalad7/",
    "org/jetbrains/",
    "java/",
    "javax/",
    "jdk/",
    "sun/",
    "it/unimi/",
    "com/google/",
    "org/apache/",
    "org/slf4j/",
    "org/joml/",
    "com/mojang/",
    "io/netty/",
    "org/lwjgl/",
];

/// Builds a census over a jar's classes.
///
/// `own_names` are the binary names of classes inside this jar, so a
/// hierarchy walk can stop at the first game class.
pub fn census<'a>(classes: impl Iterator<Item = &'a ClassFile>) -> Census {
    let all: Vec<&ClassFile> = classes.collect();
    let own: BTreeSet<&str> = all.iter().map(|c| c.name.as_str()).collect();
    let mut census = Census {
        classes: all.len(),
        ..Default::default()
    };

    for class in &all {
        // Entry points.
        for (descriptor, label) in ENTRY_POINT_ANNOTATIONS {
            if class.annotations.contains(*descriptor) {
                census
                    .entry_points
                    .insert(format!("{label} ({})", class.name));
            }
        }
        for interface in &class.interfaces {
            for (name, label) in ENTRY_POINT_INTERFACES {
                if interface.starts_with(name) {
                    census
                        .entry_points
                        .insert(format!("{label} ({})", class.name));
                }
            }
        }

        // Mixins.
        if class.annotations.contains("Lorg/spongepowered/asm/mixin/Mixin;") {
            census.mixin_classes.insert(class.name.clone());
        }
        for annotation in &class.all_annotations {
            if let Some((_, label)) = MIXIN_MEMBER_ANNOTATIONS
                .iter()
                .find(|(descriptor, _)| annotation == descriptor)
            {
                if *label == "@Mixin" {
                    continue; // counted once, on the class
                }
                census.mixin_annotation_count += 1;
                census
                    .behaviour
                    .insert(format!("{label} in {}", class.name));
            }
        }

        // What this class *is*: follow the chain to the first game class.
        if let Some(base) = game_base(class, &own) {
            census.game_extends.insert(base.clone());
            let behaviour = behaviour_of(&base);
            if let Some(behaviour) = behaviour {
                // Behaviour only counts when the class overrides something;
                // an empty subclass is still a declaration the port loses.
                let overrides = class
                    .methods
                    .iter()
                    .filter(|m| !m.name.starts_with('<') && !m.is_static() && !m.is_abstract())
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>();
                census.declares_classes_with_behaviour += 1;
                let methods = if overrides.is_empty() {
                    "no methods of its own".to_owned()
                } else {
                    format!("overrides {}", overrides.join(", "))
                };
                census.behaviour.insert(format!(
                    "{} ({}, from {}: {methods})",
                    behaviour,
                    class.name,
                    base.rsplit('/').next().unwrap_or(&base)
                ));
            }
        }

        // Registries and libraries.
        for reference in &class.class_refs {
            if is_registry(reference) {
                census.registries.insert(reference.clone());
            } else if is_library(reference) && !own.contains(reference.as_str()) {
                census.libraries.insert(library_root(reference));
            }
        }
        for reference in class.method_refs.iter().map(|r| &r.owner) {
            if is_registry(reference) {
                census.registries.insert(reference.clone());
            } else if is_library(reference) && !own.contains(reference.as_str()) {
                census.libraries.insert(library_root(reference));
            }
        }
    }
    census
}

/// The first game class a class inherits from, directly or through one of its
/// own superclasses.
///
/// A class's parent lives in another class file, so a full walk needs the
/// whole jar; [`census`] has it and this helper takes the one hop that carries
/// almost all real cases (mod classes extend a game class directly, or extend
/// another mod class that does). A class whose parent chain leaves the jar is
/// reported as "unknown base" rather than silently ignored.
fn game_base(class: &ClassFile, own: &BTreeSet<&str>) -> Option<String> {
    if let Some(parent) = &class.super_name {
        if parent.starts_with("net/minecraft/") {
            return Some(parent.clone());
        }
        if own.contains(parent.as_str()) {
            // Another class in this jar; it is walked when its turn comes,
            // and a class whose parent is in the jar but never reaches a game
            // class simply reports nothing.
        }
    }
    class
        .interfaces
        .iter()
        .find(|i| i.starts_with("net/minecraft/"))
        .cloned()
}

/// What a game superclass means for a port. Returning `Some` says "this class
/// has behaviour Java implements and a content mod cannot".
fn behaviour_of(base: &str) -> Option<&'static str> {
    let simple = base.rsplit('/').next()?.split('$').next()?;
    let label = match simple {
        "Block" | "BaseEntityBlock" | "BlockBehaviour" => "block with custom behaviour",
        "Item" => "item with custom behaviour",
        "BlockEntity" | "BlockEntityType" => "block entity (needs a block entity type)",
        "Entity" | "LivingEntity" | "PathfinderMob" | "Mob" => "entity",
        "Screen" | "ContainerScreen" | "AbstractContainerScreen" => "client GUI screen",
        "MenuType" | "AbstractContainerMenu" => "container menu",
        "Feature" | "StructureFeature" | "StructureType" => "world generation feature",
        "RecipeSerializer" | "Recipe" => "custom recipe type",
        "Enchantment" | "MobEffect" | "Potion" => "gameplay registry with behaviour",
        "Fluid" | "FlowingFluid" => "fluid with behaviour",
        "ParticleOptions" | "ParticleType" => "particle type",
        "SoundEvent" => "sound event registration",
        "ItemStack" | "DataComponentType" => "data component",
        "EntityRenderer" | "BlockEntityRenderer" => "client renderer",
        "ModelLayerLocation" | "EntityModel" => "entity model",
        _ => return None,
    };
    Some(label)
}

fn is_registry(name: &str) -> bool {
    name.starts_with("net/minecraft/core/registries/")
        || name.starts_with("net/minecraft/core/Registry")
        || name.starts_with("net/neoforged/neoforge/registries/")
        || name.starts_with("net/neoforged/neoforge/event/RegisterEvent")
        || name.starts_with("net/minecraftforge/registries/")
        || name.ends_with("DeferredRegister")
        || name.ends_with("RegistryObject")
}

fn is_library(name: &str) -> bool {
    if name.starts_with("net/minecraft/") {
        return false;
    }
    if LOADER_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return false;
    }
    name.contains('/')
}

/// The top two segments of a package, so many classes from one library appear
/// once: `com/simibubi/create/...` -> `com/simibubi`.
fn library_root(name: &str) -> String {
    let parts: Vec<&str> = name.split('/').collect();
    let take = parts.len().min(3);
    parts[..take.saturating_sub(1)].join("/")
}

/// Human-readable labels for well-known third-party modding libraries, so the
/// report says "GeckoLib animations" rather than `software/bernie/geckolib`.
pub fn describe_library(root: &str) -> Option<&'static str> {
    Some(match root {
        "software/bernie" => "GeckoLib (animated models and entities)",
        "com/simibubi" => "Create (its own API and rendering)",
        "dev/architectury" => "Architectury (multi-loader abstraction)",
        "dev/emi" | "dev/emi/emi" => "EMI (recipe viewer)",
        "me/shedaniel" => "REI/Architectury API surfaces",
        "vazkii/botania" => "Botania internals",
        "net/mehvahdjukaar" => "Moonlight library",
        "com/teamabnormals" => "Abnormals blueprint library",
        "net/createmod" => "Create internals",
        "foundry/veil" => "Veil rendering",
        "com/terraformersmc" => "Terraformers library",
        "org/jetbrains/annotations" => "annotations only",
        "net/fabricmc/fabric/api" => "Fabric API modules",
        "net/neoforged/neoforge" => "NeoForge API",
        _ => return None,
    })
}

/// Reads `*.mixins.json` / `*.mixins.json5` configuration text for its
/// `package` and `mixins`/`client`/`server` lists, which name classes that
/// must exist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MixinConfig {
    /// The config file's name inside the jar.
    pub file: String,
    pub package: Option<String>,
    pub mixins: Vec<String>,
    pub client: Vec<String>,
    pub server: Vec<String>,
    /// `refmap`, which names an obfuscation mapping file.
    pub refmap: Option<String>,
}

impl MixinConfig {
    /// Every mixin class the config names, fully qualified.
    pub fn classes(&self) -> Vec<String> {
        let prefix = self.package.as_deref().map(|p| format!("{}/", p.replace('.', "/")));
        self.mixins
            .iter()
            .chain(&self.client)
            .chain(&self.server)
            .map(|name| match &prefix {
                Some(prefix) => format!("{prefix}{}", name.replace('.', "/")),
                None => name.replace('.', "/"),
            })
            .collect()
    }
}

/// Summarises the mixin configs found in a jar.
pub fn summarize_mixins(configs: &[MixinConfig]) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for config in configs {
        out.insert(
            config.file.clone(),
            config
                .classes()
                .into_iter()
                .map(|class| {
                    format!(
                        "{class}{}",
                        config
                            .refmap
                            .as_deref()
                            .map(|r| format!(" (refmap {r})"))
                            .unwrap_or_default()
                    )
                })
                .collect(),
        );
    }
    out
}

#[cfg(test)]
mod tests;
