//! Tests build real class files byte by byte (JVMS §4), because no JDK is
//! available to compile fixtures and because hand-built files can express
//! exactly the shapes that matter: a Fabric entry point, a NeoForge `@Mod`, a
//! Mixin with `@Inject`, a class extending `Block` with overrides.

use super::*;

/// A minimal but valid class-file writer for tests.
#[derive(Default)]
struct ClassBuilder {
    pool: Vec<Vec<u8>>,
    /// Index of each UTF-8 constant, so identical strings are not duplicated.
    utf8: BTreeMap<String, u16>,
    methods: Vec<(u16, u16, u16, Vec<u8>)>,
    fields: Vec<(u16, u16, u16)>,
    class_annotations: Vec<u8>,
    access: u16,
    this: Option<String>,
    super_class: Option<String>,
    interfaces: Vec<String>,
}

impl ClassBuilder {
    fn utf8(&mut self, text: &str) -> u16 {
        if let Some(index) = self.utf8.get(text) {
            return *index;
        }
        let mut entry = vec![1u8];
        entry.extend_from_slice(&(text.len() as u16).to_be_bytes());
        entry.extend_from_slice(text.as_bytes());
        self.pool.push(entry);
        let index = self.pool.len() as u16;
        self.utf8.insert(text.to_owned(), index);
        index
    }

    fn class(&mut self, name: &str) -> u16 {
        let name_at = self.utf8(name);
        let mut entry = vec![7u8];
        entry.extend_from_slice(&name_at.to_be_bytes());
        self.pool.push(entry);
        self.pool.len() as u16
    }

    fn named(&mut self, access: u16, name: &str, super_name: &str) -> &mut Self {
        self.access = access;
        self.this = Some(name.to_owned());
        self.super_class = Some(super_name.to_owned());
        self
    }

    fn implements(&mut self, interface: &str) -> &mut Self {
        self.interfaces.push(interface.to_owned());
        self
    }

    /// Adds a method with one runtime-visible annotation of the given type
    /// descriptor and no elements.
    fn method_annotated(&mut self, access: u16, name: &str, descriptor: &str, annotation: &str) -> &mut Self {
        self.method_with(access, name, descriptor, Some(annotation))
    }

    fn method(&mut self, access: u16, name: &str, descriptor: &str) -> &mut Self {
        self.method_with(access, name, descriptor, None)
    }

    fn method_with(&mut self, access: u16, name: &str, descriptor: &str, annotation: Option<&str>) -> &mut Self {
        let name_at = self.utf8(name);
        let descriptor_at = self.utf8(descriptor);
        let mut attributes = Vec::new();
        let mut attribute_count = 0u16;
        if let Some(annotation) = annotation {
            let annotation_at = self.utf8(annotation);
            let attribute_name = self.utf8("RuntimeVisibleAnnotations");
            attribute_count += 1;
            // annotations_count, type_index, pairs_count
            let mut body = Vec::new();
            body.extend_from_slice(&1u16.to_be_bytes());
            body.extend_from_slice(&annotation_at.to_be_bytes());
            body.extend_from_slice(&0u16.to_be_bytes());
            attributes.extend_from_slice(&attribute_name.to_be_bytes());
            attributes.extend_from_slice(&(body.len() as u32).to_be_bytes());
            attributes.extend_from_slice(&body);
        }
        self.methods.push((access, name_at, descriptor_at, attributes));
        // The attribute count must precede the attributes; store it by
        // prefixing the buffer with a placeholder the writer fixes up.
        let last = self.methods.len() - 1;
        let mut buffer = attribute_count.to_be_bytes().to_vec();
        buffer.extend_from_slice(&self.methods[last].3);
        self.methods[last].3 = buffer;
        self
    }

    fn field(&mut self, access: u16, name: &str, descriptor: &str) -> &mut Self {
        let name_at = self.utf8(name);
        let descriptor_at = self.utf8(descriptor);
        self.fields.push((access, name_at, descriptor_at));
        self
    }

    fn class_annotation(&mut self, annotation: &str) -> &mut Self {
        let annotation_at = self.utf8(annotation);
        if self.class_annotations.is_empty() {
            self.class_annotations.extend_from_slice(&1u16.to_be_bytes());
        } else {
            let count = u16::from_be_bytes([
                self.class_annotations[0],
                self.class_annotations[1],
            ]) + 1;
            self.class_annotations[0..2].copy_from_slice(&count.to_be_bytes());
        }
        self.class_annotations.extend_from_slice(&annotation_at.to_be_bytes());
        self.class_annotations.extend_from_slice(&0u16.to_be_bytes());
        self
    }

    /// References another class so it appears in the constant pool, which is
    /// how a real class names its dependencies.
    fn reference(&mut self, name: &str) -> &mut Self {
        self.class(name);
        self
    }

    fn build(&mut self) -> Vec<u8> {
        // Intern every name the writer needs *before* the pool is serialised,
        // otherwise the index would point past the pool.
        let annotations_name = if self.class_annotations.is_empty() {
            0
        } else {
            self.utf8("RuntimeVisibleAnnotations")
        };
        let this_at = self.class(&self.this.clone().unwrap());
        let super_name = self.super_class.clone().unwrap();
        let super_at = if super_name.is_empty() {
            0
        } else {
            self.class(&super_name)
        };
        let interface_ats: Vec<u16> = self
            .interfaces
            .clone()
            .iter()
            .map(|i| self.class(i))
            .collect();

        let mut out = Vec::new();
        out.extend_from_slice(&0xCAFE_BABEu32.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes()); // minor
        out.extend_from_slice(&52u16.to_be_bytes()); // Java 8
        out.extend_from_slice(&((self.pool.len() + 1) as u16).to_be_bytes());
        for entry in &self.pool {
            out.extend_from_slice(entry);
        }
        out.extend_from_slice(&self.access.to_be_bytes());
        out.extend_from_slice(&this_at.to_be_bytes());
        out.extend_from_slice(&super_at.to_be_bytes());
        out.extend_from_slice(&(interface_ats.len() as u16).to_be_bytes());
        for at in interface_ats {
            out.extend_from_slice(&at.to_be_bytes());
        }
        out.extend_from_slice(&(self.fields.len() as u16).to_be_bytes());
        for (access, name, descriptor) in &self.fields {
            out.extend_from_slice(&access.to_be_bytes());
            out.extend_from_slice(&name.to_be_bytes());
            out.extend_from_slice(&descriptor.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
        }
        out.extend_from_slice(&(self.methods.len() as u16).to_be_bytes());
        for (access, name, descriptor, attributes) in &self.methods {
            out.extend_from_slice(&access.to_be_bytes());
            out.extend_from_slice(&name.to_be_bytes());
            out.extend_from_slice(&descriptor.to_be_bytes());
            out.extend_from_slice(attributes);
        }
        if self.class_annotations.is_empty() {
            out.extend_from_slice(&0u16.to_be_bytes());
        } else {
            out.extend_from_slice(&1u16.to_be_bytes());
            out.extend_from_slice(&annotations_name.to_be_bytes());
            out.extend_from_slice(&(self.class_annotations.len() as u32).to_be_bytes());
            out.extend_from_slice(&self.class_annotations);
        }
        out
    }
}

#[test]
fn reads_a_plain_class() {
    let bytes = ClassBuilder::default()
        .named(0x0021, "com/example/Thing", "java/lang/Object")
        .method(0x0001, "<init>", "()V")
        .method(0x0009, "helper", "(I)I")
        .field(0x001A, "COUNT", "I")
        .build();
    let class = parse(&bytes).unwrap();
    assert_eq!(class.name, "com/example/Thing");
    assert_eq!(class.super_name.as_deref(), Some("java/lang/Object"));
    assert_eq!(class.major_version, 52);
    assert_eq!(class.methods.len(), 2);
    assert!(class.methods[1].is_static());
    assert_eq!(class.fields[0].name, "COUNT");
}

#[test]
fn refuses_non_class_files() {
    let error = parse(b"not a class").unwrap_err();
    assert!(error.message.contains("magic"));
    // A zlib stream, the classic mix-up when someone hands over a jar entry
    // list instead of a class.
    assert!(parse(&[0x78, 0x9C, 0x03]).is_err());
    assert!(parse(&[]).is_err());
}

#[test]
fn spots_a_fabric_entry_point() {
    let bytes = ClassBuilder::default()
        .named(0x0021, "com/example/ExampleMod", "java/lang/Object")
        .implements("net/fabricmc/api/ModInitializer")
        .method(0x0001, "onInitialize", "()V")
        .build();
    let class = parse(&bytes).unwrap();
    let census = census(std::iter::once(&class));
    assert!(
        census
            .entry_points
            .iter()
            .any(|e| e.contains("Fabric ModInitializer")),
        "{:?}",
        census.entry_points
    );
}

#[test]
fn spots_neoforge_mod_annotations() {
    let bytes = ClassBuilder::default()
        .named(0x0021, "com/example/Mod", "java/lang/Object")
        .class_annotation("Lnet/neoforged/fml/common/Mod;")
        .build();
    let class = parse(&bytes).unwrap();
    assert!(class.annotations.contains("Lnet/neoforged/fml/common/Mod;"));
    let census = census(std::iter::once(&class));
    assert!(census.entry_points.iter().any(|e| e.contains("NeoForge @Mod")));
}

#[test]
fn detects_mixins_and_their_members() {
    let mut builder = ClassBuilder::default();
    builder
        .named(0x0021, "com/example/mixin/BlockMixin", "java/lang/Object")
        .class_annotation("Lorg/spongepowered/asm/mixin/Mixin;")
        .method_annotated(
            0x0001,
            "onUse",
            "(Lnet/minecraft/world/level/Level;)V",
            "Lorg/spongepowered/asm/mixin/injection/Inject;",
        )
        .method_annotated(
            0x0001,
            "redirect",
            "()V",
            "Lorg/spongepowered/asm/mixin/injection/Redirect;",
        );
    let class = parse(&builder.build()).unwrap();
    let census = census(std::iter::once(&class));
    assert_eq!(census.mixin_classes.len(), 1);
    assert!(census.mixin_annotation_count >= 2, "{}", census.mixin_annotation_count);
    let behaviour: String = census.behaviour.iter().cloned().collect();
    assert!(behaviour.contains("@Inject"), "{behaviour}");
    assert!(behaviour.contains("@Redirect"), "{behaviour}");
}

#[test]
fn identifies_what_a_game_subclass_is() {
    // A block with an override: its model and texture port, its behaviour
    // does not.
    let bytes = ClassBuilder::default()
        .named(
            0x0021,
            "com/example/block/MagicOre",
            "net/minecraft/world/level/block/Block",
        )
        .method(0x0001, "useWithoutItem", "()V")
        .method(0x0004, "getShape", "()V")
        .build();
    let class = parse(&bytes).unwrap();
    let census = census(std::iter::once(&class));
    assert!(census
        .game_extends
        .contains("net/minecraft/world/level/block/Block"));
    let behaviour: String = census.behaviour.iter().cloned().collect();
    assert!(behaviour.contains("block with custom behaviour"), "{behaviour}");
    assert!(behaviour.contains("useWithoutItem"), "names the override: {behaviour}");
    assert_eq!(census.declares_classes_with_behaviour, 1);
}

#[test]
fn finds_registries_and_third_party_libraries() {
    let mut builder = ClassBuilder::default();
    builder
        .named(0x0021, "com/example/Registrar", "java/lang/Object")
        .reference("net/minecraft/core/registries/BuiltInRegistries")
        .reference("net/neoforged/neoforge/registries/DeferredRegister")
        .reference("software/bernie/geckolib/core/animatable/AnimatableInstanceCache")
        .reference("com/simibubi/create/api/contraption/MountedStorage")
        .reference("java/util/List");
    let class = parse(&builder.build()).unwrap();
    let census = census(std::iter::once(&class));
    assert!(census
        .registries
        .iter()
        .any(|r| r.contains("BuiltInRegistries")));
    assert!(census.registries.iter().any(|r| r.contains("DeferredRegister")));
    assert!(census.libraries.contains("software/bernie"), "{:?}", census.libraries);
    assert!(census.libraries.contains("com/simibubi"), "{:?}", census.libraries);
    // JDK and loader packages are not "libraries needing reimplementation".
    assert!(!census.libraries.iter().any(|l| l.starts_with("java")));
    assert!(!census.libraries.iter().any(|l| l.starts_with("net/neoforged")));
    assert!(describe_library("software/bernie").unwrap().contains("GeckoLib"));
}

#[test]
fn reads_modified_utf8() {
    // NUL as C0 80 and a supplementary character as a surrogate pair, both of
    // which a plain UTF-8 decoder refuses.
    assert_eq!(decode_modified_utf8(&[0x41, 0xC0, 0x80, 0x42]), "A\u{0}B");
    assert_eq!(
        decode_modified_utf8(&[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]),
        "😀"
    );
    assert_eq!(decode_modified_utf8("héllo".as_bytes()), "héllo");
}

#[test]
fn reads_a_jar_manifest_main_class() {
    let manifest = "Manifest-Version: 1.0\r\nMain-Class: com.example.ExampleMod\r\n\r\n";
    assert_eq!(main_class(manifest).as_deref(), Some("com/example/ExampleMod"));
    assert_eq!(main_class("Manifest-Version: 1.0\n"), None);
}

#[test]
fn summarises_mixin_configs() {
    let config = MixinConfig {
        file: "example.mixins.json".into(),
        package: Some("com.example.mixin".into()),
        mixins: vec!["BlockMixin".into()],
        client: vec!["HandRendererMixin".into()],
        server: vec![],
        refmap: Some("example.refmap.json".into()),
    };
    assert_eq!(
        config.classes(),
        vec![
            "com/example/mixin/BlockMixin".to_owned(),
            "com/example/mixin/HandRendererMixin".to_owned()
        ]
    );
    let summary = summarize_mixins(&[config]);
    assert!(summary["example.mixins.json"][0].contains("refmap"));
}

#[test]
fn tolerates_a_truncated_class_without_panicking() {
    let full = ClassBuilder::default()
        .named(0x0021, "a/B", "java/lang/Object")
        .method(0x0001, "x", "()V")
        .build();
    for cut in 0..full.len() {
        // Every prefix either parses or errors; none panics.
        let _ = parse(&full[..cut]);
    }
    assert!(parse(&full).is_ok());
}
