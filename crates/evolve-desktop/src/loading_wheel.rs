//! Pixel-aligned vector reconstruction of the supplied loading_wheel.gfx.
//! Original source tag structure: DefineShape IDs 1, 3, 6; DefineSprite 5
//! (40 frames, inner rotation), DefineSprite 8 (65 frames, outer pulses),
//! and exported loading_symbol (ID 9). Both nested timelines run at 30 fps.
//! All transform and opacity keyframes below come from the GFX (16.16 / 8.8).
//! No eased approximation or synthesized motion is involved.
use iced::widget::svg;

pub const FPS: u32 = 30;
pub const LOOP_FRAMES: u32 = 520; // LCM of the two authored independent timelines.

// 16.16 SWF matrix coefficients [a, b, c, d] at each ShowFrame.
const ROTATION: [[i32; 4]; 40] = [
    [65536, 0, 0, 65536],
    [62248, -20290, 20290, 62248],
    [52714, -38761, 38761, 52714],
    [38070, -53216, 53216, 38070],
    [19477, -62505, 62505, 19477],
    [-638, -65524, 65524, -638],
    [-21097, -61970, 61970, -21097],
    [-39441, -52200, 52200, -39441],
    [-53704, -37370, 37370, -53704],
    [-62751, -18660, 18660, -62751],
    [-65502, 1490, -1490, -65502],
    [-61682, 21903, -21903, -61682],
    [-51675, 40114, -40114, -51675],
    [-36664, 54183, -54183, -36664],
    [-17840, 62986, -62986, -17840],
    [2341, 65469, -65469, 2341],
    [22702, 61384, -61384, 22702],
    [40614, 51276, -51276, 40614],
    [54652, 35950, -35950, 54652],
    [63210, 17017, -17017, 63210],
    [65424, -3192, 3192, 65424],
    [61076, -23497, 23497, 61076],
    [50738, -41275, 41275, 50738],
    [35232, -55112, 55112, 35232],
    [16191, -63422, 63422, 16191],
    [-4042, -65368, 65368, -4042],
    [-24288, -60757, 60757, -24288],
    [-41929, -50189, 50189, -41929],
    [-55564, -34508, 34508, -55564],
    [-63574, -15572, 15572, -63574],
    [-65302, 4891, -4891, -65302],
    [-60428, 25075, -25075, -60428],
    [-49633, 42576, -42576, -49633],
    [-33775, 56007, -56007, -33775],
    [-14742, 63768, -63768, -14742],
    [5740, 65225, -65225, 5740],
    [25857, 60088, -60088, 25857],
    [43217, 49069, -49069, 43217],
    [56438, 33040, -33040, 56438],
    [63988, 13951, -13951, 63988],
];

// [first ring scale (16.16), first alpha (8.8), second scale, second alpha].
// In the source, pulse sprite 8 is 65 frames, including its blank tail.
const PULSES: [[i32; 4]; 65] = [
    [30801, 0, 0, 0],
    [31904, 10, 0, 0],
    [33155, 21, 0, 0],
    [34553, 33, 0, 0],
    [36098, 47, 0, 0],
    [37789, 62, 0, 0],
    [39628, 78, 0, 0],
    [41614, 96, 0, 0],
    [43748, 115, 0, 0],
    [46028, 135, 30801, 0],
    [48455, 157, 31522, 6],
    [51030, 180, 32449, 15],
    [53752, 204, 33581, 25],
    [56620, 229, 34921, 37],
    [59636, 256, 36465, 50],
    [61261, 221, 38216, 66],
    [62766, 188, 40173, 83],
    [64150, 158, 42335, 102],
    [65415, 131, 44703, 123],
    [66558, 106, 47278, 146],
    [67582, 84, 50059, 171],
    [68485, 64, 53045, 197],
    [69267, 47, 56238, 226],
    [69930, 33, 59636, 256],
    [70471, 21, 61261, 221],
    [70892, 12, 62766, 188],
    [71194, 5, 64150, 158],
    [71375, 1, 65415, 131],
    [71434, 0, 66558, 106],
    [0, 0, 67582, 84],
    [0, 0, 68485, 64],
    [0, 0, 69267, 47],
    [0, 0, 69930, 33],
    [0, 0, 70471, 21],
    [0, 0, 70892, 12],
    [0, 0, 71194, 5],
    [0, 0, 71375, 1],
    [0, 0, 71434, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
];

// The original DefineShape curves, in twips, not polygon approximations.
const BLACK_RING: &str = "M681 -681 Q963 -399 963 0 Q963 399 681 681 Q399 963 0 963 Q-399 963 -681 681 Q-963 399 -963 0 Q-963 -399 -681 -681 Q-399 -963 0 -963 Q399 -963 681 -681 Z M388 -389 Q549 -228 549 0 Q549 227 388 388 Q227 549 0 549 Q-227 549 -388 388 Q-549 227 -549 0 Q-549 -228 -388 -389 Q-227 -549 0 -549 Q227 -549 388 -389 Z";
const SOLID_HALF: &str = "M0 857 Q-356 857 -607 606 Q-858 355 -858 0 Q-858 -355 -607 -606 Q-356 -857 0 -857 L0 -684 Q-284 -684 -484 -484 Q-684 -283 -684 0 Q-684 283 -484 483 Q-284 684 0 684 L0 857 Z";
const GRADIENT_HALF: &str = "M0 857 Q355 857 606 606 Q857 355 857 0 Q857 -355 606 -606 Q355 -857 0 -857 L0 -684 Q283 -684 483 -484 Q683 -283 683 0 Q683 283 483 483 Q283 684 0 684 L0 857 Z";
const OUTER_RING: &str = "M-1 -1165 Q481 -1165 822 -825 Q1164 -482 1164 0 Q1164 482 822 822 Q481 1164 -1 1164 Q-483 1164 -825 822 Q-1165 482 -1165 0 Q-1165 -482 -825 -825 Q-483 -1165 -1 -1165 Z";

fn fixed(raw: i32) -> f64 { f64::from(raw) / 65536.0 }

/// Build one composite frame from BOTH original nested movie timelines.
/// dim = 1.0 in the footer, 0.12 as a passive workbench watermark.
pub fn svg_for_frame(frame: u32, dim: f64) -> String {
    let [a, b, c, d] = ROTATION[(frame % 40) as usize];
    let [s1, alpha1, s2, alpha2] = PULSES[(frame % 65) as usize];
    let mut art = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-1240 -1240 2480 2480" opacity="{dim}">
<defs><linearGradient id="sourceGradient" gradientUnits="userSpaceOnUse"
x1="-16384" y1="0" x2="16384" y2="0"
gradientTransform="matrix(0.0190887451171875 -0.020965576171875 -0.008819580078125 -0.045379638671875 37 28)">
<stop offset="0.17254901960784313" stop-color="#d02d18"/>
<stop offset="1" stop-color="#d02d18" stop-opacity="0"/>
</linearGradient></defs>
<path fill="#000000" fill-opacity="0.6484375" fill-rule="evenodd" d="{BLACK_RING}"/>
<g transform="matrix({} {} {} {} -1 0)">
<path fill="#d02d18" d="{SOLID_HALF}"/>
<path fill="url(#sourceGradient)" d="{GRADIENT_HALF}"/>
</g>"#,
        fixed(a), fixed(b), fixed(c), fixed(d)
    );
    // The pulse sprite has parent alpha 218/256. Every individual alpha
    // and scale is read from its respective original PlaceObject2 record.
    for (scale, alpha) in [(s1, alpha1), (s2, alpha2)] {
        if alpha > 0 {
            let opacity = (218.0 / 256.0) * (f64::from(alpha) / 256.0);
            art.push_str(&format!(
                r#"<g opacity="{opacity}" transform="matrix({0} 0 0 {0} -1 -1)"><path d="{OUTER_RING}" fill="none" stroke="#d02d18" stroke-width="40"/></g>"#,
                fixed(scale)
            ));
        }
    }
    art.push_str("</svg>");
    art
}

pub fn handle(frame: u32, opacity: f64) -> svg::Handle {
    svg::Handle::from_memory(svg_for_frame(frame, opacity).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_timeline_keyframes_match_exactly() {
        assert_eq!(ROTATION.len(), 40);
        assert_eq!(PULSES.len(), 65);
        assert_eq!(ROTATION[0], [65536, 0, 0, 65536]);
        assert_eq!(ROTATION[1], [62248, -20290, 20290, 62248]);
        assert_eq!(PULSES[0], [30801, 0, 0, 0]);
        assert_eq!(PULSES[14], [59636, 256, 36465, 50]);
        assert_eq!(PULSES[38], [0, 0, 0, 0]);
        assert_eq!(svg_for_frame(0, 1.0), svg_for_frame(LOOP_FRAMES, 1.0));
        assert_ne!(svg_for_frame(0, 1.0), svg_for_frame(1, 1.0));
    }
}
