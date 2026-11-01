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
@group(0) @binding(0) var hdr_scene:texture_2d<f32>;
@group(0) @binding(1) var hdr_bloom:texture_2d<f32>;
@group(0) @binding(2) var hdr_sampler:sampler;
struct CompositeParams {
    params:vec4<f32>,
    film:vec4<f32>, // authored shoulder, midtones, toe, whitepoint
    adaptation:vec4<f32>, // authored scene key, min exposure, max exposure, enabled
    shafts:vec4<f32>, // strength
}
@group(0) @binding(3) var<uniform> composite:CompositeParams;
@group(0) @binding(4) var color_chart:texture_2d<f32>;
@group(0) @binding(5) var scene_log_luminance:texture_2d<f32>;
@group(0) @binding(6) var sun_shafts:texture_2d<f32>;
// Exposure that brings the measured scene to its authored key, within limits.
fn adapted_exposure()->f32 {
    if (composite.adaptation.w<0.5) { return 1.0; }
    let average=exp(textureLoad(scene_log_luminance,vec2<i32>(0,0),0).r);
    // The game scales its target by a runtime constant that is not in its
    // shaders; this value is calibrated to in-game captures.
    return clamp(composite.adaptation.x/(average+0.00001),
        composite.adaptation.y,composite.adaptation.z)*0.85;
}
// The final-scene curve from the game's compiled HDR shader.
fn filmic_curve(x:vec3<f32>)->vec3<f32>{
    let a=0.22*composite.film.x;
    let cb=0.03*composite.film.y;
    let b=0.30*composite.film.y;
    let de=0.002*composite.film.z;
    return (x*(a*x+cb)+vec3<f32>(de))/(x*(a*x+b)+vec3<f32>(0.06))-vec3<f32>(composite.film.z*0.033333);
}
fn film_map(radiance:vec3<f32>)->vec3<f32>{
    let v=max(radiance*composite.params.y*adapted_exposure(),vec3<f32>(0.0));
    let white=max(composite.film.w,0.01);
    return clamp(filmic_curve(v)/filmic_curve(vec3<f32>(white)),vec3<f32>(0.0),vec3<f32>(1.0));
}
fn to_display(c:vec3<f32>)->vec3<f32>{
    let low=c*12.92;
    let high=pow(max(c,vec3<f32>(0.0)),vec3<f32>(1.0/2.4))*1.055-vec3<f32>(0.055);
    return select(high,low,c<vec3<f32>(0.003131));
}
fn from_display(c:vec3<f32>)->vec3<f32>{
    let low=c/12.92;
    let high=pow((max(c,vec3<f32>(0.0))+vec3<f32>(0.055))/1.055,vec3<f32>(2.4));
    return select(high,low,c<vec3<f32>(0.04045));
}
// Original charts are a 256x16 strip of 16 slices, with blue selecting the slice.
fn graded(display_in:vec3<f32>)->vec3<f32>{
    let display=clamp(display_in,vec3<f32>(0.0),vec3<f32>(1.0));
    let slice=display.b*15.0;
    let lower=floor(slice);
    let upper=min(lower+1.0,15.0);
    let cell=(display.rg*15.0+vec2<f32>(0.5))/16.0;
    let a=textureSampleLevel(color_chart,hdr_sampler,
        vec2<f32>((lower+cell.x)/16.0,cell.y),0.0).rgb;
    let b=textureSampleLevel(color_chart,hdr_sampler,
        vec2<f32>((upper+cell.x)/16.0,cell.y),0.0).rgb;
    return mix(a,b,slice-lower);
}
@fragment
fn fs_composite(input:FsOut)->@location(0) vec4<f32> {
    // Unlike the quarter-res filter, this pass draws in the UI's scissored
    // viewport. Fragment coordinates must sample the scene at absolute
    // physical pixels, not stretch the whole window into the viewport.
    let size=vec2<f32>(textureDimensions(hdr_scene,0));
    let uv=clamp(input.clip.xy/size,vec2<f32>(0.0),vec2<f32>(1.0));
    let source=max(textureSampleLevel(hdr_scene,hdr_sampler,uv,0.0).rgb,vec3<f32>(0.0));
    if (composite.params.z > 0.5) {
        // Source RGB (unlit) is an explicit original-map color diagnostic;
        // no HDR film curve or bloom must interfere with source comparison.
        return vec4<f32>(source,1.0);
    }
    let bloom=textureSampleLevel(hdr_bloom,hdr_sampler,uv,0.0).rgb;
    let shafts=textureSampleLevel(sun_shafts,hdr_sampler,uv,0.0).rgb;
    let combined=source+bloom*max(composite.params.x,0.0);
    // The game screens shafts over the display-encoded image.
    let display=to_display(film_map(combined));
    let lit=display+min(shafts*composite.shafts.x,vec3<f32>(1.0))*(vec3<f32>(1.0)-display);
    return vec4<f32>(from_display(graded(lit)),1.0);
}
