// Shared linear HDR + bounded quarter-resolution bloom for the Model Viewer
// and Level Editor. This does not embed any original game shader binaries.
struct FsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_fullscreen(@builtin(vertex_index) idx:u32)->FsOut {
    let xy=array<vec2<f32>,3>(
        vec2<f32>(-1.0,-1.0), vec2<f32>(3.0,-1.0), vec2<f32>(-1.0,3.0));
    var result:FsOut;
    result.clip=vec4<f32>(xy[idx],0.0,1.0);
    result.uv=xy[idx]*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5,0.5);
    return result;
}
@group(0) @binding(0) var source_texture:texture_2d<f32>;
@group(0) @binding(1) var source_sampler:sampler;
@group(0) @binding(2) var<uniform> filter_params:vec4<f32>;

@fragment
fn fs_extract(input:FsOut)->@location(0) vec4<f32> {
    let radiance=max(textureSampleLevel(source_texture,source_sampler,input.uv,0.0).rgb,
        vec3<f32>(0.0));
    // Smooth HDR threshold avoids a hard halo seam. Keep the original linear
    // source untouched; discard only dim photons from the bloom branch.
    let brightness=dot(radiance,vec3<f32>(0.2126,0.7152,0.0722));
    let knee=smoothstep(0.85,1.6,brightness);
    let weight=knee * max(brightness-0.8,0.0) / max(brightness,0.0001);
    return vec4<f32>(min(radiance*weight,vec3<f32>(64.0)),1.0);
}
@fragment
fn fs_blur(input:FsOut)->@location(0) vec4<f32> {
    let offsets=array<f32,5>(0.0,1.0,2.0,3.0,4.0);
    let weights=array<f32,5>(0.227027,0.1945946,0.1216216,0.054054,0.016216);
    var color=textureSampleLevel(source_texture,source_sampler,input.uv,0.0).rgb*weights[0];
    for (var i=1u;i<5u;i=i+1u) {
        let delta=filter_params.xy*offsets[i];
        color+=(textureSampleLevel(source_texture,source_sampler,input.uv+delta,0.0).rgb+
                textureSampleLevel(source_texture,source_sampler,input.uv-delta,0.0).rgb)*weights[i];
    }
    return vec4<f32>(color,1.0);
}
// Mean log luminance of the scene, for authored eye adaptation.
@fragment
fn fs_luminance(input:FsOut)->@location(0) vec4<f32> {
    var sum=0.0;
    for (var y=0;y<16;y=y+1) {
        for (var x=0;x<16;x=x+1) {
            let uv=(vec2<f32>(f32(x),f32(y))+vec2<f32>(0.5))/16.0;
            let color=textureSampleLevel(source_texture,source_sampler,uv,0.0).rgb;
            sum+=log(dot(color,vec3<f32>(0.2126,0.7152,0.0722))+0.000001);
        }
    }
    return vec4<f32>(sum/256.0,0.0,0.0,1.0);
}
// Light from open sky, smeared toward the sun. filter_params: sun uv, strength.
@fragment
fn fs_shafts(input:FsOut)->@location(0) vec4<f32> {
    if (filter_params.z<=0.0) { return vec4<f32>(0.0,0.0,0.0,1.0); }
    let to_sun=filter_params.xy-input.uv;
    var sum=vec3<f32>(0.0);
    var weight=1.0;
    var total=0.0;
    for (var i=0;i<48;i=i+1) {
        let uv=input.uv+to_sun*(f32(i)/48.0);
        if (all(uv>=vec2<f32>(0.0)) && all(uv<=vec2<f32>(1.0))) {
            let sampled=textureSampleLevel(source_texture,source_sampler,uv,0.0);
            sum+=min(sampled.rgb,vec3<f32>(8.0))*(1.0-clamp(sampled.a,0.0,1.0))*weight;
        }
        total+=weight;
        weight*=0.97;
    }
    return vec4<f32>(sum/total,1.0);
}
