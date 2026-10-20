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

@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    let base = textureSample(diffuse, material_sampler, input.uv);
    let encoded_normal = textureSample(normal_map, material_sampler, input.uv).rg;
    let glow = textureSample(emissive_map, material_sampler, input.uv).rgb;
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
    // Neutral studio lights keep source surface detail visible without
    // inventing missing diffuse artwork. Two-sided light accommodates
    // CryEngine meshes with reversed source winding in isolated previews.
    let key = normalize(vec3<f32>(0.45, -0.4, 0.8));
    let fill = normalize(vec3<f32>(-0.7, 0.25, 0.5));
    let key_amount = abs(dot(surface, key));
    let fill_amount = abs(dot(surface, fill));
    let brightness = (0.30 + 0.54 * key_amount + 0.16 * fill_amount)
        * clamp(input.light_strength, 0.1, 2.0);
    // Studio-light preview: highlight direction uses original view-space
    // normal and authored specular/gloss data, without assuming metallic PBR.
    let to_eye = normalize(vec3<f32>(-input.view_position.xy, 3.0 - input.view_position.z));
    let half_key = normalize(key + to_eye);
    let half_fill = normalize(fill + to_eye);
    let exponent = mix(6.0, 112.0, clamp(gloss, 0.0, 1.0));
    let highlight = pow(abs(dot(surface, half_key)), exponent) * 0.8
                  + pow(abs(dot(surface, half_fill)), exponent) * 0.2;
    let specular_light = specular_color * highlight * clamp(input.light_strength, 0.1, 2.0);
    // Emissive remains unlit. Do not multiply specular color into the
    // diffuse texture or use its alpha as inferred gloss.
    return vec4<f32>(base.rgb * brightness + glow + specular_light, base.a);
}
