use std::{io, path::Path};

use log::debug;
use registry::{
    Image as RemoteImage, Manifest as RemoteManifest, ManifestLayer as RemoteLayer,
    Registry as RemoteRegistry, Tag as RemoteTag,
};
use types::{Architecture, Digest, OciBootstrapError};

use crate::{
    container::ContainerSpec,
    local::{LocalImage, LocalLayer, LocalManifest, LocalRegistry, LocalTag},
    utils::get_current_oci_os,
};

pub(crate) trait Registry {
    fn find_image_by_name<'a>(
        &'a self,
        name: &str,
    ) -> Result<Option<Box<dyn Image + 'a>>, OciBootstrapError>;
}

impl Registry for RemoteRegistry {
    fn find_image_by_name<'a>(
        &'a self,
        name: &str,
    ) -> Result<Option<Box<dyn Image + 'a>>, OciBootstrapError> {
        Ok(Some(Box::new(self.image(name)?)))
    }
}

impl Registry for LocalRegistry {
    fn find_image_by_name<'a>(
        &'a self,
        name: &str,
    ) -> Result<Option<Box<dyn Image + 'a>>, OciBootstrapError> {
        Ok(if let Some(image) = self.image_by_name(name) {
            Some(Box::new(image))
        } else {
            None
        })
    }
}

pub(crate) fn get_registry(
    spec: &ContainerSpec,
) -> Result<Option<Box<dyn Registry>>, OciBootstrapError> {
    Ok(Some(if let Some(domain) = &spec.domain {
        match domain.as_str() {
            "localhost" => Box::new(LocalRegistry::new()?),
            d => Box::new(RemoteRegistry::connect(d)?),
        }
    } else {
        return Ok(None);
    }))
}

pub(crate) trait Image {
    fn name(&self) -> &str;
    fn tags<'a>(&'a self) -> Result<Vec<Box<dyn Tag + 'a>>, OciBootstrapError>;

    fn find_tag_by_name<'a>(
        &'a self,
        tag_name: &str,
    ) -> Result<Option<Box<dyn Tag + 'a>>, OciBootstrapError> {
        Ok(self.tags()?.into_iter().find(|t| t.name() == tag_name))
    }

    fn latest<'a>(&'a self) -> Result<Option<Box<dyn Tag + 'a>>, OciBootstrapError> {
        self.find_tag_by_name("latest")
    }
}

impl PartialEq<String> for dyn Image {
    fn eq(&self, other: &String) -> bool {
        self.name().eq(other)
    }
}

impl PartialEq<str> for dyn Image {
    fn eq(&self, other: &str) -> bool {
        self.name().eq(other)
    }
}

impl<T> Image for Box<T>
where
    T: Image + ?Sized,
{
    fn name(&self) -> &str {
        (**self).name()
    }

    fn tags<'a>(&'a self) -> Result<Vec<Box<dyn Tag + 'a>>, OciBootstrapError> {
        (**self).tags()
    }
}

impl Image for RemoteImage<'_> {
    fn name(&self) -> &str {
        self.name()
    }

    fn tags<'a>(&'a self) -> Result<Vec<Box<dyn Tag + 'a>>, OciBootstrapError> {
        let tags = self.tags()?;

        let mut vec: Vec<Box<dyn Tag>> = Vec::with_capacity(tags.len());
        for tag in tags.into_iter() {
            vec.push(Box::new(tag));
        }

        Ok(vec)
    }
}

impl Image for LocalImage<'_> {
    fn name(&self) -> &str {
        self.name()
    }

    fn tags<'a>(&'a self) -> Result<Vec<Box<dyn Tag + 'a>>, OciBootstrapError> {
        let tags = self.tags();

        let mut vec: Vec<Box<dyn Tag>> = Vec::with_capacity(tags.len());
        for tag in tags.into_iter() {
            vec.push(Box::new(tag));
        }

        Ok(vec)
    }
}

pub(crate) trait Tag {
    fn name(&self) -> &str;
    fn manifest_for_platform<'a>(
        &'a self,
        arch: Architecture,
        os: &str,
    ) -> Result<Option<Box<dyn Manifest + 'a>>, OciBootstrapError>;

    fn manifest<'a>(&'a self) -> Result<Option<Box<dyn Manifest + 'a>>, OciBootstrapError> {
        self.manifest_for_platform(Architecture::default(), get_current_oci_os())
    }
}

impl PartialEq<String> for dyn Tag {
    fn eq(&self, other: &String) -> bool {
        self.name().eq(other)
    }
}

impl PartialEq<str> for dyn Tag {
    fn eq(&self, other: &str) -> bool {
        self.name().eq(other)
    }
}

impl<T> Tag for Box<T>
where
    T: Tag + ?Sized,
{
    fn name(&self) -> &str {
        (**self).name()
    }

    fn manifest_for_platform<'a>(
        &'a self,
        arch: Architecture,
        os: &str,
    ) -> Result<Option<Box<dyn Manifest + 'a>>, OciBootstrapError> {
        (**self).manifest_for_platform(arch, os)
    }
}

impl Tag for RemoteTag<'_> {
    fn name(&self) -> &str {
        self.name()
    }

    fn manifest_for_platform(
        &'_ self,
        arch: Architecture,
        os: &str,
    ) -> Result<Option<Box<dyn Manifest + '_>>, OciBootstrapError> {
        Ok(Some(Box::new(self.manifest_for_config(arch, os)?)))
    }
}

impl Tag for LocalTag<'_> {
    fn name(&self) -> &str {
        self.name()
    }

    fn manifest_for_platform(
        &'_ self,
        arch: Architecture,
        os: &str,
    ) -> Result<Option<Box<dyn Manifest + '_>>, OciBootstrapError> {
        if let Some(manifest) = self.manifest_for_platform(arch, os)? {
            return Ok(Some(Box::new(manifest)));
        } else {
            return Ok(None);
        }
    }
}

pub(crate) trait Manifest {
    fn layers(&self) -> Vec<Box<dyn Layer>>;
}

impl<T> Manifest for Box<T>
where
    T: Manifest + ?Sized,
{
    fn layers(&self) -> Vec<Box<dyn Layer>> {
        (**self).layers()
    }
}

impl Manifest for RemoteManifest<'_> {
    fn layers(&self) -> Vec<Box<dyn Layer>> {
        todo!()
    }
}

impl Manifest for LocalManifest<'_> {
    fn layers(&self) -> Vec<Box<dyn Layer>> {
        todo!()
    }
}

pub(crate) trait Layer {
    fn digest(&self) -> Digest;
    fn reader(&self) -> Box<dyn io::Read>;

    fn extract(&self, _output: &Path) -> Result<(), io::Error> {
        todo!()
    }
}

impl PartialEq<String> for dyn Layer {
    fn eq(&self, other: &String) -> bool {
        self.digest().to_raw_string().eq(other)
    }
}

impl PartialEq<str> for dyn Layer {
    fn eq(&self, other: &str) -> bool {
        self.digest().to_raw_string().eq(other)
    }
}

impl<T> Layer for Box<T>
where
    T: Layer + ?Sized,
{
    fn digest(&self) -> Digest {
        (**self).digest()
    }

    fn reader(&self) -> Box<dyn io::Read> {
        (**self).reader()
    }
}

impl Layer for RemoteLayer<'_> {
    fn digest(&self) -> Digest {
        todo!()
    }

    fn reader(&self) -> Box<dyn io::Read> {
        todo!()
    }
}

impl Layer for LocalLayer<'_> {
    fn digest(&self) -> Digest {
        todo!()
    }

    fn reader(&self) -> Box<dyn io::Read> {
        todo!()
    }
}
