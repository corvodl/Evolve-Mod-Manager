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
// x is the material's emission strength, y is the user's viewport slider.
// Source values, texture masks, diffuse and FBX export remain unchanged.
@group(0) @binding(7) var<uniform> emission_params: vec4<f32>;
// Separate, optional armor overlay: this channel is black when unchecked,
// independently of any source-authored Emittance/GlowAmount texture.
@group(0) @binding(8) var armor_glow_map: texture_2d<f32>;
// Independent authored Decal UV transform: x/y tiling, z/w animation offset.
// Only the emission map uses these values; other maps stay at mesh UV0.
@group(0) @binding(9) var<uniform> emissive_uv: vec4<f32>;
// Authored Specular with Shininess as smoothness, Diffuse tint, Emissive color
// with a flag for a decal the glow pass multiplies by diffuse.
@group(0) @binding(10) var<uniform> material: array<vec4<f32>, 3>;

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

// The game's light pass: GGX over its smoothness, with Schlick Fresnel. A surface
// with no specular at all gets no grazing sheen either.
fn game_specular(n:vec3<f32>,to_view:vec3<f32>,to_light:vec3<f32>,f0:vec3<f32>,smoothness:f32)->vec3<f32> {
  let h=normalize(to_light+to_view);
  let no_l=max(dot(n,to_light),0.0);
  let no_v=max(dot(n,to_view),0.0);
  let no_h=max(dot(n,h),0.0);
  let rough=1.0-clamp(smoothness,0.0,1.0)*0.7;
  let rough2=rough*rough;
  let rough6=rough2*rough2*rough2;
  let alpha2=max(rough6*rough6,0.00001);
  let c=rough6*0.5+0.8;
  let k=c*c*0.5;
  let visibility=max((no_v*(1.0-k)+k)*(no_l*(1.0-k)+k),0.000001);
  let d=no_h*no_h*(alpha2-1.0)+1.0;
  let edge=1.0-max(dot(to_light,h),0.0);
  let fresnel=f0+(vec3<f32>(1.0)-f0)*edge*edge*edge*edge*edge;
  return fresnel*step(0.001,max(f0.r,max(f0.g,f0.b)))*(alpha2/(d*d)/visibility*0.25*no_l);
}
fn shade(input: Output) -> vec4<f32> {
    let base = textureSample(diffuse, material_sampler, input.uv);
    let normal_sample = textureSample(normal_map, material_sampler, input.uv);
    let encoded_normal = normal_sample.rg;
    let decal_coord = input.uv * emissive_uv.xy + emissive_uv.zw;
    let glow = textureSample(emissive_map, material_sampler, decal_coord).rgb;
    let armor_glow = textureSample(armor_glow_map, material_sampler, input.uv).rgb;
    // Specular is source reflectance color, NOT a gloss/roughness scalar.
    // Only an explicitly authored Gloss/Glossiness/Smoothness map controls
    // the highlight width. Missing specular uses a black neutral fallback.
    let specular_color = textureSample(specular_map, material_sampler, input.uv).rgb * material[0].rgb;
    // A ddna normal map carries smoothness in alpha; other formats read as 1.
    let gloss = textureSample(gloss_map, material_sampler, input.uv).r * normal_sample.a * material[0].a;

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
    // Either side of a two-sided surface can face the camera in this rig.
    let facing = select(-surface, surface, dot(surface, to_eye) >= 0.0);
    let highlight = 0.84 * game_specular(facing, to_eye, key, specular_color, gloss)
                  + 0.16 * game_specular(facing, to_eye, fill, specular_color, gloss);
    let specular_light = highlight * exposure;
    // The game's emission, unscaled: see authored_emission_strength. The final
    // bloom and tone curve bound the display value.
    let decal_base = mix(vec3<f32>(1.0), base.rgb * base.a, material[2].a);
    let source_glow = glow * emission_params.x * emission_params.y * decal_base;
    let viewport_glow = min(source_glow, vec3<f32>(4096.0));

    // Optional Stage 2 armor-glow overlay has no authored strength: a neutral
    // one-unit source that never inherits the active material's. Disabled
    // overlays stay black.
    let armor_source = armor_glow * emission_params.y;
    let armor_viewport = min(armor_source, vec3<f32>(32.0));
    return vec4<f32>(base.rgb * material[1].rgb * (irradiance * exposure + material[2].rgb)
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
    if leaf_alpha < emission_params.w {
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
