struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) view_position: vec3<f32>,
    @location(3) light_strength: f32,
}
@group(0) @binding(0) var diffuse: texture_2d<f32>;
@group(0) @binding(1) var material_sampler: sampler;
@group(0) @binding(2) var normal_map: texture_2d<f32>;
@group(0) @binding(3) var emissive_map: texture_2d<f32>;
@group(0) @binding(4) var specular_map: texture_2d<f32>;
@group(0) @binding(5) var gloss_map: texture_2d<f32>;
// Explicit authored opacity map. Diffuse alpha may hold CryEngine gloss/mask
// data, so it is never reused for a separate opacity-reference material.
@group(0) @binding(6) var opacity_map: texture_2d<f32>;
// x is the material's original GlowAmount, y is the user's viewport slider.
// z is universal 100/3 legacy-Qt calibration times Kala's optional, verified
// equipment-only state correction (Banshee 0.3, Armor Reducer 4).
// Source values, texture masks, diffuse and FBX export remain unchanged.
@group(0) @binding(7) var<uniform> emission_params: vec4<f32>;
// Separate, optional armor overlay: this channel is black when unchecked,
// independently of any source-authored Emittance/GlowAmount texture.
@group(0) @binding(8) var armor_glow_map: texture_2d<f32>;
// Independent authored Decal UV transform: x/y tiling, z/w animation offset.
// Only the emission map uses these values; other maps stay at mesh UV0.
@group(0) @binding(9) var<uniform> emissive_uv: vec4<f32>;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) view_position: vec3<f32>,
    @location(4) light_strength: f32,
) -> Output {
    var out: Output;
    out.position = vec4<f32>(position, 1.0);
    out.uv = uv;
    out.normal = normal;
    out.view_position = view_position;
    out.light_strength = light_strength;
    return out;
}

fn shade(input: Output) -> vec4<f32> {
    let base = textureSample(diffuse, material_sampler, input.uv);
    let encoded_normal = textureSample(normal_map, material_sampler, input.uv).rg;
    let decal_coord = input.uv * emissive_uv.xy + emissive_uv.zw;
    let glow = textureSample(emissive_map, material_sampler, decal_coord).rgb;
    let armor_glow = textureSample(armor_glow_map, material_sampler, input.uv).rgb;
    // Specular is source reflectance color, NOT a gloss/roughness scalar.
    // Only an explicitly authored Gloss/Glossiness/Smoothness map controls
    // the highlight width. Missing specular uses a black neutral fallback.
    let specular_color = textureSample(specular_map, material_sampler, input.uv).rgb;
    let gloss = textureSample(gloss_map, material_sampler, input.uv).r;

    // CryEngine BC5 stores tangent-space XY. Reconstruct non-negative Z,
    // without treating the normal texture as sRGB. Missing maps use flat XY.
    let xy = encoded_normal * 2.0 - vec2<f32>(1.0, 1.0);
    let tangent_normal = normalize(vec3<f32>(
        xy, sqrt(max(0.0, 1.0 - dot(xy, xy)))
    ));
    // Compiled meshes can omit a usable per-vertex normal stream. Use real
    // view-space triangle derivatives rather than flattening every face into
    // the same clay silhouette. The original geometry is not modified.
    let face_normal = cross(dpdx(input.view_position), dpdy(input.view_position));
    var surface = input.normal;
    if length(surface) < 0.00001 {
        surface = face_normal;
    }
    if length(surface) < 0.00001 {
        surface = vec3<f32>(0.0, 0.0, 1.0);
    }
    surface = normalize(surface);

    // Derivative-based tangent frame respects unwrapped/repeating original UV0,
    // including mirror signs. Do not mutate the source mesh or original UVs.
    let dpdx_pos = dpdx(input.view_position);
    let dpdy_pos = dpdy(input.view_position);
    let dpdx_uv = dpdx(input.uv);
    let dpdy_uv = dpdy(input.uv);
    let determinant = dpdx_uv.x * dpdy_uv.y - dpdx_uv.y * dpdy_uv.x;
    if abs(determinant) > 0.000001 {
        let tangent_raw = (dpdx_pos * dpdy_uv.y - dpdy_pos * dpdx_uv.y) / determinant;
        let bitangent_raw = (dpdy_pos * dpdx_uv.x - dpdx_pos * dpdy_uv.x) / determinant;
        let tangent = tangent_raw - surface * dot(surface, tangent_raw);
        let bitangent = bitangent_raw - surface * dot(surface, bitangent_raw);
        if length(tangent) > 0.00001 && length(bitangent) > 0.00001 {
            let t = normalize(tangent);
            let b = normalize(bitangent - t * dot(t, bitangent));
            surface = normalize(t * tangent_normal.x + b * tangent_normal.y + surface * tangent_normal.z);
        }
    }
    // CryEngine-inspired preview lighting: warm directional key/sun,
    // cool sky fill, lower hemisphere bounce, and a subtle rear rim light.
    // It is a material-inspection rig, not a claim to reproduce the game's
    // actual deferred lighting, time-of-day, GI, SSAO or shadow cascades.
    // Two-sided contribution keeps historical 2012 CryTek winding visible.
    let key = normalize(vec3<f32>(0.48, -0.46, 0.75));
    let fill = normalize(vec3<f32>(-0.62, 0.26, 0.54));
    let back = normalize(vec3<f32>(-0.2, 0.78, 0.55));
    let key_diffuse = abs(dot(surface, key));
    let fill_diffuse = abs(dot(surface, fill));
    let back_diffuse = max(dot(surface, back), 0.0);
    let sky = clamp(surface.z * 0.5 + 0.5, 0.0, 1.0);
    let ambient = mix(vec3<f32>(0.20, 0.18, 0.17),
                      vec3<f32>(0.30, 0.33, 0.37), sky);
    let irradiance = ambient
        + vec3<f32>(0.58, 0.52, 0.45) * key_diffuse
        + vec3<f32>(0.15, 0.19, 0.25) * fill_diffuse
        + vec3<f32>(0.13, 0.15, 0.19) * pow(back_diffuse, 2.0);
    let exposure = clamp(input.light_strength, 0.2, 4.0);
    // Original CryEngine-style spec/gloss slots remain authoritative.
    // No missing source specular is invented: an absent map stays black.
    let to_eye = normalize(vec3<f32>(-input.view_position.xy, 3.0 - input.view_position.z));
    let half_key = normalize(key + to_eye);
    let half_fill = normalize(fill + to_eye);
    let exponent = mix(8.0, 120.0, clamp(gloss, 0.0, 1.0));
    let highlight = 0.84 * pow(abs(dot(surface, half_key)), exponent)
                  + 0.16 * pow(abs(dot(surface, half_fill)), exponent);
    let specular_light = specular_color * highlight * exposure;
    // Emissive is independent of incident light. The original public Qt
    // manager interprets GlowAmount 3 as 100 visualization units for EVERY
    // source material, not just Kala. Scale only true authored emission.
    // Per-pixel peak rolloff keeps high-range RGB from clipping to white
    // in the 8-bit preview; it does not invent bloom outside the mesh.
    let source_glow = glow * emission_params.x * emission_params.y;
    let calibrated_glow = source_glow * emission_params.z;
    let peak = max(max(calibrated_glow.r, calibrated_glow.g), calibrated_glow.b);
    let viewport_glow = calibrated_glow / (1.0 + peak);

    // Optional Stage 2 armor-glow overlay has no authored GlowAmount.
    // Treat it as a separate neutral one-unit source using the same 3=>100
    // preview calibration, never the active material's GlowAmount or Kala
    // state multiplier. Disabled overlays stay black.
    let armor_source = armor_glow * emission_params.y;
    let armor_calibrated = armor_source * (100.0 / 3.0);
    let armor_peak = max(max(armor_calibrated.r, armor_calibrated.g), armor_calibrated.b);
    let armor_viewport = armor_calibrated / (1.0 + armor_peak);
    return vec4<f32>(base.rgb * irradiance * exposure
        + viewport_glow
        + armor_viewport + specular_light, base.a);
}

@fragment
fn fs_opaque(input: Output) -> @location(0) vec4<f32> {
    // A disabled GPU blend alone does not make the offscreen wgpu target
    // opaque: copying diffuse alpha into that target can still create holes
    // when Iced composites the viewport with the rest of the interface.
    let color = shade(input);
    return vec4<f32>(color.rgb, 1.0);
}

@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    // WGPU opaque mode disregards alpha entirely. For glass/water this is
    // the authored diffuse-alpha translucency entrypoint.
    return shade(input);
}

// Solid CryEngine character masks are alpha-tested with depth writes.
// A gray authored opacity mask must not make the entire body see-through.
// Preserve true blending for authored glass/water materials.
@fragment
fn fs_masked(input: Output) -> @location(0) vec4<f32> {
    let coverage = clamp(textureSample(opacity_map, material_sampler, input.uv).r, 0.0, 1.0);
    if coverage < 0.5 {
        discard;
    }
    let color = shade(input);
    return vec4<f32>(color.rgb, 1.0);
}

// Leaf cards in the standalone Model Viewer share the same original
// Vegetation material semantics as the instanced Level Editor: diffuse.a
// clips geometry with depth writes. Do not use the separate SSS/Opacity map
// for coverage or alpha-blend a solid leaf card.
@fragment
fn fs_foliage(input: Output) -> @location(0) vec4<f32> {
    let leaf_alpha = textureSample(diffuse, material_sampler, input.uv).a;
    if leaf_alpha < 0.5 {
        discard;
    }
    let color = shade(input);
    return vec4<f32>(color.rgb, 1.0);
}

@fragment
fn fs_opacity(input: Output) -> @location(0) vec4<f32> {
    let color = shade(input);
    // Use the explicitly authored opacity texture in linear space. This
    // source controls transparency independently of the diffuse DDS alpha.
    let opacity = clamp(textureSample(opacity_map, material_sampler, input.uv).r, 0.0, 1.0);
    if opacity <= 0.001 {
        discard;
    }
    return vec4<f32>(color.rgb, opacity);
}

// The floor uses a separate untextured line pipeline but the *same*
// source-space camera projection/depth as the mesh. It cannot alter assets.
struct FloorOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}
@vertex
fn vs_floor(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
) -> FloorOutput {
    var out: FloorOutput;
    out.position = vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs_floor(input: FloorOutput) -> @location(0) vec4<f32> {
    return input.color;
}
