use wgpu::naga::{front::wgsl, valid::{Capabilities, ValidationFlags, Validator}};

#[test]
fn software_clip_shader_validates_without_native_half_unpacking() {
    let shader = "@group(0) @binding(0) var<storage, read_write> values: array<u32>;
@compute @workgroup_size(1) fn run() { values[0] = bitcast<u32>(unpack2x16float(values[0]).x); }";
    let validate = |source: &str| {
        let module = wgsl::parse_str(source).unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        Validator::new(ValidationFlags::all(), Capabilities::default() - Capabilities::SHADER_FLOAT16_IN_FLOAT32)
            .validate(&module)
    };
    assert!(validate(shader).is_err());
    validate(&super::software_source(shader)).unwrap();
}
