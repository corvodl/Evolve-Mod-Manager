struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
}
@group(0) @binding(0) var diffuse: texture_2d<f32>;
@group(0) @binding(1) var diffuse_sampler: sampler;
@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
) -> Output {
    var out: Output;
    out.position = vec4<f32>(position, 1.0);
    out.uv = uv;
    out.normal = normal;
    return out;
}
@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    let base = textureSample(diffuse, diffuse_sampler, input.uv);
    let n = normalize(input.normal);
    let light = normalize(vec3<f32>(0.35, -0.3, 0.9));
    let brightness = 0.30 + 0.70 * max(dot(n, light), 0.0);
    return vec4<f32>(base.rgb * brightness, base.a);
}
