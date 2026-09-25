use std::path::Path;
use std::sync::Arc;
use wallrs_proto::WallpaperManifest;

use crate::{RendererError, WallpaperRenderer};

/// Capabilities declared by a wallpaper renderer implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RendererCapabilities {
    /// Whether this renderer can consume PipeWire frequency spectrum analysis.
    pub needs_audio_spectrum: bool,
    /// Whether this renderer produces audio output itself (e.g. video player).
    pub produces_audio: bool,
}

/// Factory trait responsible for creating, validating, and describing renderer plugins.
pub trait RendererFactory: Send + Sync {
    /// Constructs a boxed renderer instance from a parsed wallpaper manifest and its root directory.
    fn create_renderer(
        &self,
        manifest: &WallpaperManifest,
        base_dir: &Path,
    ) -> Result<Box<dyn WallpaperRenderer>, RendererError>;

    /// Returns true if this factory supports the specified wallpaper type name (e.g. "image", "shader", "video").
    fn supports_type(&self, type_name: &str) -> bool;

    /// Validates the wallpaper manifest and associated assets without instantiating full GPU resources.
    fn validate(
        &self,
        _manifest: &WallpaperManifest,
        _base_dir: &Path,
    ) -> Result<(), RendererError> {
        Ok(())
    }

    /// Queries capabilities for a given wallpaper manifest.
    fn capabilities(&self, _manifest: &WallpaperManifest) -> RendererCapabilities {
        RendererCapabilities::default()
    }
}

/// Thread-safe registry holding registered `RendererFactory` plugins.
#[derive(Default, Clone)]
pub struct RendererRegistry {
    factories: Vec<Arc<dyn RendererFactory>>,
}

impl std::fmt::Debug for RendererRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererRegistry")
            .field("factories_count", &self.factories.len())
            .finish()
    }
}

impl RendererRegistry {
    /// Creates an empty `RendererRegistry`.
    pub fn new() -> Self {
        Self {
            factories: Vec::new(),
        }
    }

    /// Registers a new renderer factory.
    pub fn register<F>(&mut self, factory: F)
    where
        F: RendererFactory + 'static,
    {
        self.factories.push(Arc::new(factory));
    }

    /// Registers an `Arc`-wrapped renderer factory.
    pub fn register_arc(&mut self, factory: Arc<dyn RendererFactory>) {
        self.factories.push(factory);
    }

    /// Returns the first factory that supports the given type name, if any.
    pub fn get(&self, type_name: &str) -> Option<Arc<dyn RendererFactory>> {
        self.factories
            .iter()
            .find(|f| f.supports_type(type_name))
            .cloned()
    }

    /// Returns true if any registered factory supports the given type name.
    pub fn is_supported(&self, type_name: &str) -> bool {
        self.factories.iter().any(|f| f.supports_type(type_name))
    }

    /// Returns the number of registered factories.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Returns whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SolidColorRenderer;

    struct DummyFactory;

    impl RendererFactory for DummyFactory {
        fn create_renderer(
            &self,
            _manifest: &WallpaperManifest,
            _base_dir: &Path,
        ) -> Result<Box<dyn WallpaperRenderer>, RendererError> {
            Ok(Box::new(SolidColorRenderer::new([0.0, 0.0, 0.0, 1.0])))
        }

        fn supports_type(&self, type_name: &str) -> bool {
            type_name == "dummy"
        }

        fn capabilities(&self, _manifest: &WallpaperManifest) -> RendererCapabilities {
            RendererCapabilities {
                needs_audio_spectrum: true,
                produces_audio: false,
            }
        }
    }

    #[test]
    fn test_registry_registration_and_lookup() {
        let mut registry = RendererRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry.register(DummyFactory);
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.is_supported("dummy"));
        assert!(!registry.is_supported("shader"));

        let factory = registry.get("dummy").expect("dummy factory should exist");
        assert!(factory.supports_type("dummy"));
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = DummyFactory;
        let manifest = WallpaperManifest::from_toml_str(
            r#"
            [wallpaper]
            name = "test"
            type = "dummy"
            "#,
        )
        .unwrap();

        let caps = factory.capabilities(&manifest);
        assert!(caps.needs_audio_spectrum);
        assert!(!caps.produces_audio);
    }
}
