"""Source-backed scalar used by Evolve Terrain.Layer, for Blender/offline baking.

This is the recovered GetLayerBlendingValue operation from the authenticated
D3D11/ShadeLib.cfib parsed shader representation. Terrain.cfxb calls it with
bumpHeightMapSampler, pPass.fAlpha, BlendFactor, BlendFalloff and true.

It is NOT a reconstruction of pPass.fAlpha from categorical LayerId pixels,
and not a complete native terrain shader. Callers must provide genuine
per-pixel alpha and the matching material height sample.
"""
import math


class TerrainBlendInputsMissing(ValueError):
    pass


def _finite(value, name):
    if not isinstance(value, (int, float)) or not math.isfinite(value):
        raise TerrainBlendInputsMissing(name + " must be a finite source value")
    return float(value)


def _sat(value):
    return min(max(value, 0.0), 1.0)



def terrain_pass_alpha(interpolated_source_channel, ambient_opacity):
    """Recover Terrain.cfxb's pre-blend alpha arithmetic, not its CPU input.

    Terrain.cfxb: pPass.fAlpha = saturate(pPass.IN[0x103][0x1fe]
                                        - AmbientOpPB[0x200]).
    0x103/0x1fe designate the packed vertex-to-fragment interpolator
    component in the parsed shader stream; the original terrain vertex
    generator and corresponding D3D11 input layout are NOT verified.
    Values must be extracted from an authenticated matching draw. Do not
    substitute categorical LayerIds, neighboring coverage, or fake vertex alpha.
    """
    raw = _finite(interpolated_source_channel, "terrain interpolator")
    offset = _finite(ambient_opacity, "AmbientOpPB alpha")
    return _sat(raw - offset)


def get_layer_blending_value(
    height_red, vertex_alpha, blend_factor, blend_falloff, *,
    has_source_height=False, has_source_alpha=False,
):
    """Evaluate Evolve's recovered terrain scalar; fail closed on missing inputs.

    The original shader uses a texture sample and shader-passed alpha. Neither
    is equivalent to nearest-neighbor LayerId or an arbitrarily assigned 0.5.
    """
    if not has_source_height or not has_source_alpha:
        raise TerrainBlendInputsMissing(
            "Native terrain blend requires sampled BumpHeight red and pPass.fAlpha; "
            "categorical LayerId coverage alone is insufficient"
        )
    h = _finite(height_red, "BumpHeight red")
    alpha = _finite(vertex_alpha, "pPass.fAlpha")
    factor = _finite(blend_factor, "BlendFactor")
    falloff = _finite(blend_falloff, "BlendFalloff")
    if falloff <= 0.0 or factor < -1.0:
        raise TerrainBlendInputsMissing("unsupported BlendFactor/BlendFalloff")
    blend_map = _sat(_sat(alpha * 2.0 - 1.0) + h)
    blend_fac = alpha * blend_map * (1.0 + factor)
    return _sat(blend_fac ** falloff) if blend_fac > 0.0 else 0.0


def bake_terrain_layer_color(
    original_macro_rgb, detail_rgb, detail_strength,
    height_red, vertex_alpha, blend_factor, blend_falloff, *,
    has_source_height=False, has_source_alpha=False,
):
    """Bake ONE verified layer scalar over source macro RGB, no synthetic masks.

    Multi-layer ordering and actual high-pass compositing require independent
    validation against the target Evolve shader variant. This is an opt-in
    building block, not automatically applied to categorical FBX materials.
    """
    weight = get_layer_blending_value(
        height_red, vertex_alpha, blend_factor, blend_falloff,
        has_source_height=has_source_height, has_source_alpha=has_source_alpha,
    )
    strength = _finite(detail_strength, "detail strength")
    if len(original_macro_rgb) != 3 or len(detail_rgb) != 3:
        raise TerrainBlendInputsMissing("RGB values must have three channels")
    original = [_finite(x, "macro color") for x in original_macro_rgb]
    detail = [_finite(x, "detail color") for x in detail_rgb]
    layered = [_sat(original[i] + (detail[i] - 0.5) * strength) for i in range(3)]
    return tuple(original[i] * (1.0 - weight) + layered[i] * weight for i in range(3))
