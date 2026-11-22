
fn rune_clip_fade_word(word: u32) -> f32 {
    let sign = (word & 0x8000u) << 16u;
    let exponent = (word >> 10u) & 31u;
    let fraction = word & 0x3ffu;
    if exponent == 0u {
        let subnormal = f32(fraction) * 0.000000059604644775390625;
        return bitcast<f32>(sign | bitcast<u32>(subnormal));
    }
    let wide_exponent = select(exponent + 112u, 255u, exponent == 31u);
    return bitcast<f32>(sign | (wide_exponent << 23u) | (fraction << 13u));
}

fn rune_clip_fade_pair(bits: u32) -> vec2<f32> {
    return vec2<f32>(rune_clip_fade_word(bits), rune_clip_fade_word(bits >> 16u));
}
