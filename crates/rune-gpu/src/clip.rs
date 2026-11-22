use std::borrow::Cow;

/// WGSL for a shader that unpacks Rune's binary16 clip fades.
pub fn clip_shader<'a>(_device: &wgpu::Device, source: impl Into<Cow<'a, str>>) -> wgpu::ShaderSource<'a> {
    let source = source.into();
    #[cfg(target_os = "linux")]
    {
        let adapter = _device.adapter_info();
        if adapter.backend == wgpu::Backend::Vulkan && adapter.device_type == wgpu::DeviceType::Cpu {
            return wgpu::ShaderSource::Wgsl(software_source(&source).into());
        }
    }
    wgpu::ShaderSource::Wgsl(source)
}

#[cfg(any(target_os = "linux", test))]
fn software_source(source: &str) -> String {
    // Lavapipe cannot unpack binary16 on CPUs without native half-float conversion.
    let mut source = source.replace("unpack2x16float(", "rune_clip_fade_pair(");
    source.push_str(include_str!("clip_fade_pair.wgsl"));
    source
}

#[cfg(test)]
#[path = "clip_tests.rs"]
mod tests;
