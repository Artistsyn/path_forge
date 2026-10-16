//! What every scene field is for an editor: its slider range, unit and whether it is Advanced
//! (`META`, which the schema cannot say), the scene's JSON Schema indexed for lookups (`Schema`),
//! and `form`, the whole scene as one description an editor builds itself from. The desktop
//! studio's inspector and the phone apps (through `mobile/`) both read these, so a field added to
//! the scene appears in all of them.

use serde_json::{json, Map, Value};

/// Range, unit and visibility for fields matched by path ("*" matches a list index;
/// a pattern starting with "*." matches that suffix anywhere).
pub struct Meta { pub pat: &'static str, pub min: f64, pub max: f64, pub unit: &'static str, pub adv: bool }

const fn m(pat: &'static str, min: f64, max: f64, unit: &'static str) -> Meta { Meta { pat, min, max, unit, adv: false } }
const fn a(pat: &'static str, min: f64, max: f64, unit: &'static str) -> Meta { Meta { pat, min, max, unit, adv: true } }

pub const META: &[Meta] = &[
    m("canvas.width", 64.0, 2048.0, " px"), m("canvas.height", 64.0, 4096.0, " px"),
    m("camera.eye_height", 0.2, 10.0, " m"), m("camera.horizon", 0.02, 0.98, ""), m("camera.zoom", 0.3, 4.0, "×"), m("camera.lens_curve", -1.0, 1.0, ""),
    m("camera.moves.bob", 0.0, 0.2, " m"), m("camera.moves.stride", 0.3, 2.0, " m"), m("camera.moves.sway", 0.0, 5.0, "°"), m("camera.moves.pan", -30.0, 30.0, "°"), m("camera.moves.pans", 1.0, 8.0, ""),
    m("camera.moves.tilt", -15.0, 15.0, "°"), m("camera.moves.tilts", 1.0, 8.0, ""), m("camera.moves.dolly", -10.0, 20.0, " m"), m("camera.moves.dolly_focus", 2.0, 40.0, " m"), m("camera.moves.dollies", 1.0, 8.0, ""),
    m("camera.moves.fly_in", 0.0, 12.0, " s"), m("camera.moves.fly_height", 3.0, 50.0, " m"),
    m("path.half_width", 0.2, 10.0, " m"), m("path.flare", 0.0, 0.95, ""), m("path.bend", -2.0, 2.0, ""), m("ground_cover.*.density", 0.0, 1.0, ""), m("ground_cover.*.height", 0.02, 1.5, " m"), m("ground_cover.*.patchiness", 0.0, 1.0, ""), m("path.waterway.frozen", 0.0, 1.0, ""), m("path.waterway.cascades", 0.0, 1.0, ""), m("path.waterway.cascade_spacing", 6.0, 120.0, " m"), m("path.winding.amplitude", 0.0, 8.0, " m"), m("path.winding.length", 10.0, 200.0, " m"), m("path.winding.turn", 0.0, 1.0, ""), m("path.winding.irregular", 0.0, 1.0, ""), m("path.winding.phase", 0.0, 1.0, ""), m("path.hill", -2.0, 2.0, ""),
    m("path.edge_noise", 0.0, 1.0, ""), m("path.edge_dark", 0.0, 1.0, ""),
    m("*.material.tile_size", 0.2, 10.0, " m"), m("*.material.brightness", 0.2, 3.0, "×"), m("*.material.damage", 0.0, 1.0, ""),
    a("*.material.noise", 0.0, 60.0, ""), a("*.material.rotate", 0.0, 1.0, ""),
    m("verge.tuft_density", 0.0, 4.0, "×"), m("verge.tuft_height", 0.02, 1.5, " m"),
    m("walls.gap", 0.0, 20.0, " m"), m("walls.height", 0.0, 20.0, " m"), m("walls.base_shadow", 0.0, 1.0, ""),
    m("ceiling.height", 1.0, 20.0, " m"),
    m("*.radius", 0.0, 0.4, ""), m("*.intensity", 0.0, 5.0, "×"), m("sky.moon.phase", -1.0, 1.0, ""), m("sky.moon.opacity", 0.0, 1.0, ""), m("sky.moon.relief", 0.0, 2.0, ""), m("sky.moon.seed", 0.0, 9999.0, ""),
    m("sky.meteors.count", 0.0, 40.0, " per loop"), m("sky.meteors.duration", 0.2, 3.0, " s"), m("sky.meteors.length", 0.05, 1.0, ""),
    m("sky.meteors.angle", -180.0, 180.0, "°"), m("sky.meteors.spread", 0.0, 180.0, "°"), m("sky.meteors.brightness", 0.0, 3.0, "×"), m("sky.meteors.tint", 0.0, 1.0, ""), m("sky.meteors.seed", 0.0, 9999.0, ""),
    m("sky.stars.count", 0.0, 1000.0, ""), m("sky.stars.size", 0.2, 4.0, "×"), m("sky.stars.twinkle", 0.0, 1.0, ""),
    m("sky.clouds.count", 0.0, 60.0, ""), m("sky.clouds.cover", 0.0, 1.0, ""), m("sky.clouds.shade", 0.0, 1.0, ""), m("sky.cloud_layers.*.cover", 0.0, 1.0, ""), m("sky.cloud_layers.*.shade", 0.0, 1.0, ""), m("sky.cloud_layers.*.drift", 0.0, 5.0, "×"), m("sky.cloud_layers.*.scale", 0.2, 4.0, "×"), m("sky.cloud_layers.*.opacity", 0.0, 1.0, ""), m("sky.cloud_layers.*.variation", 0.0, 1.0, ""), m("sky.clouds.drift", 0.0, 5.0, "×"), m("sky.clouds.scale", 0.2, 4.0, "×"), m("sky.clouds.opacity", 0.0, 1.0, ""), a("sky.clouds.variation", 0.0, 1.0, ""),
    m("sky.clouds.patches", 0.0, 1.0, ""), m("sky.cloud_layers.*.patches", 0.0, 1.0, ""),
    m("sky.cloud_mix.layers", 1.0, 4.0, ""), m("sky.cloud_mix.cover", 0.0, 1.0, ""), m("sky.cloud_mix.patches", 0.0, 1.0, ""), m("sky.cloud_mix.variety", 0.0, 1.0, ""),
    m("sky.cloud_mix.drift", 0.0, 5.0, "×"), m("sky.cloud_mix.opacity", 0.0, 1.0, ""), m("sky.cloud_mix.seed", 0.0, 9999.0, ""),
    m("sky.sunset.amount", 0.0, 1.0, ""), m("sky.sunset.glow", 0.0, 2.0, ""), m("sky.sunset.height", 0.05, 1.0, ""), m("sky.sunset.spread", 0.1, 3.0, ""),
    m("sky.sunset.clouds", 0.0, 1.0, ""), m("sky.sunset.light", 0.0, 1.0, ""),
    m("waterfalls.*.lateral", 0.0, 20.0, " m"), m("waterfalls.*.spacing", 10.0, 200.0, " m"), m("waterfalls.*.offset", 0.0, 200.0, " m"), m("waterfalls.*.height", 1.0, 30.0, " m"),
    m("waterfalls.*.width", 0.3, 10.0, " m"), m("waterfalls.*.spray", 0.0, 1.0, ""), m("waterfalls.*.rainbow", 0.0, 1.0, ""),
    m("wildlife.birds.flocks", 0.0, 8.0, ""), m("wildlife.birds.count", 1.0, 30.0, ""), m("wildlife.birds.size", 0.005, 0.1, ""), m("wildlife.birds.height", 0.0, 1.0, ""),
    m("wildlife.birds.flap", 0.5, 8.0, " /s"), m("wildlife.birds.crossing", 3.0, 60.0, " s"), m("wildlife.bats.count", 0.0, 40.0, ""), m("wildlife.bats.size", 0.1, 1.0, " m"),
    m("wildlife.fish.spacing", 1.0, 40.0, " m"), m("wildlife.fish.leaps", 1.0, 6.0, " per loop"), m("wildlife.fish.size", 0.1, 1.5, " m"),
    m("season.season", 0.0, 4.0, ""), m("season.years_per_loop", 0.0, 4.0, " per loop"),
    m("sky.day_night.hour", 0.0, 24.0, " h"), m("sky.day_night.days_per_loop", 0.0, 4.0, " per loop"), m("sky.day_night.sunrise", 0.0, 24.0, " h"),
    m("sky.day_night.sunset", 0.0, 24.0, " h"), m("sky.day_night.sun_height", 0.1, 2.0, ""), m("sky.day_night.glow", 0.0, 2.0, ""),
    m("sky.day_night.keep_look", 0.0, 1.0, ""), m("sky.day_night.night_light", 0.0, 1.0, ""),
    m("light.ambient", 0.0, 2.0, ""), m("light.fog.distance", 2.0, 300.0, " m"), m("light.bands", 0.0, 8.0, ""),
    m("fixtures.*.height", 0.0, 6.0, " m"), m("fixtures.*.spacing", 0.5, 48.0, " m"), m("fixtures.*.offset", 0.0, 48.0, " m"),
    m("fixtures.*.lateral", -3.0, 5.0, " m"), m("fixtures.*.size", 0.2, 4.0, "×"), m("fixtures.*.radius", 0.5, 30.0, " m"),
    m("fixtures.*.flicker", 0.0, 3.0, "×"), m("fixtures.*.jitter", 0.0, 3.0, " m"),
    m("fixtures.*.jitter_along", 0.0, 1.0, ""), m("fixtures.*.height_var", 0.0, 3.0, " m"), m("fixtures.*.size_var", 0.0, 0.9, ""),
    m("fixtures.*.intensity_var", 0.0, 1.0, ""), m("fixtures.*.color_var", 0.0, 1.0, ""), m("fixtures.*.density", 0.0, 1.0, ""),
    m("fixtures.*.posts_between", 0.0, 7.0, ""), m("fixtures.*.string_lights.bulbs", 1.0, 40.0, ""), m("fixtures.*.string_lights.sag", 0.0, 2.0, " m"), m("fixtures.*.string_lights.brightness", 0.0, 3.0, "×"), m("fixtures.*.string_lights.size", 0.2, 3.0, "×"), m("fixtures.*.string_lights.twinkle", 0.0, 1.0, ""), m("weather.snow_cover.props", 0.0, 1.0, ""), m("weather.snow_cover.ground", 0.0, 1.0, ""),
    m("fixtures.*.cluster", 1.0, 8.0, ""), m("fixtures.*.stagger", 0.0, 1.0, ""), m("fixtures.*.set_piece", 0.0, 7.0, ""),
    m("props.*.jitter_along", 0.0, 1.0, ""), m("props.*.stagger", 0.0, 1.0, ""), m("props.*.lean_var", 0.0, 0.2, ""),
    m("props.*.orbit.radius", 0.5, 30.0, " m"), m("props.*.orbit.radius_var", 0.0, 10.0, " m"), m("props.*.orbit.height", 0.0, 20.0, " m"),
    m("props.*.orbit.twist", -6.0, 6.0, ""), m("props.*.orbit.spin", -4.0, 4.0, ""), m("props.*.orbit.stray", 0.0, 0.5, ""),
    m("props.*.lateral", -3.0, 20.0, " m"), m("props.*.spacing", 0.3, 48.0, " m"), m("props.*.offset", 0.0, 48.0, " m"),
    m("props.*.rows", 1.0, 8.0, ""), m("props.*.row_spacing", 0.5, 20.0, " m"), m("props.*.density", 0.0, 1.0, ""),
    m("props.*.scale", 0.1, 6.0, "×"), m("props.*.scale_var", 0.0, 1.0, ""), m("props.*.tint_var", 0.0, 0.5, ""), m("props.*.shadow_opacity", 0.0, 1.0, ""),
    a("props.*.jitter", 0.0, 5.0, " m"), a("props.*.sink", 0.0, 1.0, " m"),
    m("set_pieces.*.spacing", 4.0, 96.0, " m"), m("set_pieces.*.offset", 0.0, 96.0, " m"), m("set_pieces.*.width", 0.0, 12.0, " m"),
    m("set_pieces.*.height", 1.0, 10.0, " m"), m("set_pieces.*.shadow", 0.0, 1.0, ""),
    m("path.stairs.spacing", 2.0, 96.0, " m"), m("path.stairs.steps", 1.0, 40.0, ""), m("path.stairs.rise", 0.05, 0.4, " m"),
m("path.stairs.run", 0.15, 1.0, " m"), m("path.stairs.offset", 0.0, 96.0, " m"),
m("path.bridge.spacing", 4.0, 96.0, " m"), m("path.bridge.length", 0.5, 64.0, " m"), m("path.bridge.offset", 0.0, 96.0, " m"),
m("path.bridge.depth", 0.5, 120.0, " m"), m("path.bridge.rail_height", 0.2, 2.0, " m"), m("path.bridge.water_level", 0.1, 120.0, " m"),
m("path.fork.spacing", 4.0, 96.0, " m"), m("path.fork.offset", 0.0, 96.0, " m"), m("path.fork.angle", 5.0, 85.0, "°"),
m("path.fork.half_width", 0.2, 4.0, " m"), m("path.fork.depth", 0.5, 30.0, " m"), m("path.fork.height", 0.5, 6.0, " m"),
m("path.waterway.flow", -8.0, 8.0, ""), m("path.waterway.foam", 0.0, 1.0, ""), m("path.waterway.foam_width", 0.01, 2.0, " m"), m("path.waterway.lap", 0.0, 12.0, ""),
m("path.waterway.wet", 0.0, 2.0, " m"), m("path.waterway.float_density", 0.0, 1.0, ""), m("path.waterway.float_size", 0.05, 2.0, " m"),
m("*.gloss", 0.0, 1.0, ""), m("*.ripples", 0.0, 1.0, ""),
m("weather.lightning.strikes", 0.0, 16.0, " per loop"), m("weather.lightning.intensity", 0.0, 4.0, "×"),
m("weather.fog_banks.spacing", 4.0, 96.0, " m"), m("weather.fog_banks.length", 0.5, 48.0, " m"), m("weather.fog_banks.density", 0.0, 3.0, "/m"),
m("weather.fog_banks.offset", 0.0, 96.0, " m"),
m("weather.precipitation.intensity", 0.0, 1.0, ""), m("weather.precipitation.size", 0.2, 4.0, "×"),
m("weather.precipitation.wetness", 0.0, 1.0, ""), m("weather.precipitation.puddles", 0.0, 1.0, ""), m("weather.precipitation.splashes", 0.0, 1.0, ""),
m("weather.precipitation.cover", 0.0, 1.0, ""), m("weather.precipitation.track", 0.0, 1.0, ""), m("weather.precipitation.haze", 0.0, 1.0, ""),
m("weather.drips.rate", 0.0, 10.0, " /m"),
m("weather.wind.speed", -20.0, 20.0, " m/s"), m("weather.wind.gusts", 0.0, 1.0, ""), m("weather.wind.sway", 0.0, 1.0, ""), m("weather.wind.flutter", 0.0, 1.0, ""),
m("weather.sandstorm.intensity", 0.0, 1.0, ""),
m("weather.mist.height", 0.1, 5.0, " m"), m("weather.mist.density", 0.0, 2.0, " /m"), m("weather.mist.patchiness", 0.0, 1.0, ""), m("weather.mist.wisps", 0.0, 1.0, ""), m("weather.mist.soft", 0.0, 1.0, ""), m("weather.mist.spread", 0.0, 10.0, " m"), m("weather.mist.glow", 0.0, 2.0, ""), m("weather.mist.from", 0.0, 150.0, " m"),
m("weather.light_shafts.sun", 0.0, 2.0, ""), m("weather.light_shafts.lamps", 0.0, 2.0, ""),
m("weather.heat_shimmer.strength", 0.0, 1.0, ""), m("weather.heat_shimmer.speed", 0.0, 3.0, "×"),
m("weather.lens.amount", 0.0, 1.0, ""), m("weather.lens.size", 0.3, 3.0, "×"), m("weather.lens.trails", 0.0, 1.0, ""), m("weather.lens.blur", 0.0, 1.0, ""),
m("sky.clouds.shadows", 0.0, 1.0, ""),
m("sky.rainbow.saturation", 0.0, 1.0, ""), m("sky.halo.size", 0.05, 1.0, ""), m("sky.halo.strength", 0.0, 2.0, ""),
m("sky.aurora.intensity", 0.0, 2.0, ""), m("sky.aurora.height", 0.0, 1.0, ""), m("sky.aurora.speed", 0.0, 4.0, "×"),
m("sky.aurora.tall", 0.3, 2.0, "×"), m("sky.aurora.coverage", 0.0, 1.0, ""), m("sky.aurora.rays", 0.0, 2.0, ""),
m("sky.aurora.waves", 0.0, 2.0, ""), m("sky.aurora.arc", -2.0, 4.0, ""), m("sky.aurora.edge", 0.0, 3.0, "×"),
m("sky.aurora.ground_glow", 0.0, 2.0, ""),
m("sky.space.stars", 0.0, 2.0, ""), m("sky.space.star_brightness", 0.0, 3.0, "×"), m("sky.space.nebula", 0.0, 2.0, ""), m("sky.space.nebula_scale", 0.2, 4.0, "×"),
m("sky.space.galaxy", 0.0, 2.0, ""), m("sky.space.galaxy_angle", -90.0, 90.0, "°"), m("sky.space.galaxy_width", 0.02, 1.0, ""),
m("sky.space.galaxy_height", -1.0, 1.5, ""), m("sky.space.galaxy_core", -1.0, 1.0, ""),
m("sky.space.star_glow", 0.0, 2.0, ""), m("sky.space.star_clumps", 0.0, 1.0, ""), m("sky.space.star_spikes", 0.0, 2.0, ""),
m("sky.space.twinkle", 0.0, 1.0, ""), m("sky.space.twinkle_speed", 1.0, 8.0, " per loop"), m("sky.space.nebula_dust", 0.0, 1.0, ""), m("sky.space.galaxy_dust", 0.0, 1.0, ""),
m("sky.space.objects.*.size", 0.01, 1.0, ""), m("sky.space.objects.*.angle", -180.0, 180.0, "°"), m("sky.space.objects.*.aspect", 0.1, 1.0, ""),
m("sky.space.objects.*.brightness", 0.0, 3.0, "×"), m("sky.space.objects.*.spin", -6.0, 6.0, " per loop"),
m("sky.space.planets.*.orbit.distance", 1.1, 8.0, "×"), m("sky.space.planets.*.orbit.tilt", 0.0, 90.0, "°"), m("sky.space.planets.*.orbit.roll", -90.0, 90.0, "°"),
m("sky.space.planets.*.orbit.turns", -4.0, 4.0, " per loop"), m("sky.space.planets.*.orbit.phase", 0.0, 1.0, ""), m("sky.space.planets.*.orbit.around", 0.0, 2.0, ""),
m("sky.space.planets.*.radius", 0.01, 1.5, ""), m("sky.space.planets.*.atmosphere_strength", 0.0, 2.0, ""), m("sky.space.planets.*.light_angle", -180.0, 180.0, "°"),
m("sky.space.planets.*.night", 0.0, 1.0, ""), m("sky.space.planets.*.tilt", -90.0, 90.0, "°"), m("sky.space.planets.*.spin", -4.0, 4.0, " turns"),
m("sky.space.planets.*.clouds", 0.0, 1.0, ""), m("sky.space.planets.*.city_lights", 0.0, 2.0, ""),
m("sky.space.planets.*.rings.inner", 1.0, 4.0, "×"), m("sky.space.planets.*.rings.outer", 1.0, 5.0, "×"), m("sky.space.planets.*.rings.open", 0.0, 1.0, ""),
m("sky.space.planets.*.rings.angle", -90.0, 90.0, "°"), m("sky.space.planets.*.rings.opacity", 0.0, 1.0, ""),
        m("sky.space.black_hole.size", 0.02, 0.5, ""), m("sky.space.black_hole.tilt", 0.0, 89.0, "°"), m("sky.space.black_hole.roll", -90.0, 90.0, "°"),
        m("sky.space.black_hole.disk_inner", 1.6, 10.0, " Rs"), m("sky.space.black_hole.disk_outer", 3.0, 30.0, " Rs"), m("sky.space.black_hole.brightness", 0.0, 4.0, "×"),
        m("sky.space.black_hole.doppler", 0.0, 1.0, ""), m("sky.space.black_hole.spin", -12.0, 12.0, " turns"), m("sky.space.black_hole.lensing", 0.0, 3.0, "×"),
        m("sky.space.black_hole.infall", 0.0, 48.0, ""), m("sky.space.black_hole.infall_speed", 1.0, 6.0, " trips"), m("sky.space.black_hole.light", 0.0, 3.0, ""),
        m("sky.space.black_hole.nodes", 0.0, 48.0, ""), m("sky.space.black_hole.node_size", 0.0, 0.06, ""), m("sky.space.black_hole.distance", 2.0, 100.0, "×"), m("sky.space.star_colors", 0.0, 1.0, ""),
m("companions.*.size", 0.2, 30.0, " m"), m("companions.*.bob", 0.0, 3.0, " m"), m("companions.*.bob_cycles", 0.0, 8.0, ""),
m("companions.*.weave", 0.0, 5.0, " m"), m("companions.*.weave_cycles", 0.0, 8.0, ""), m("companions.*.light", 0.0, 4.0, "×"), m("companions.*.glow_from", 0.0, 1.0, ""),
m("props.*.float", 0.0, 30.0, " m"), m("props.*.float_var", 0.0, 20.0, " m"),
m("*.material.glow", 0.0, 3.0, ""),
m("path.edge_lights.strength", 0.0, 4.0, ""), m("path.edge_lights.width", 0.005, 0.5, " m"), m("path.edge_lights.inset", -0.5, 1.0, " m"),
m("path.edge_lights.dash", 0.0, 10.0, " m"), m("path.edge_lights.flow", -20.0, 20.0, ""),
m("sky.tunnel.radius", 1.0, 40.0, " m"), m("sky.tunnel.intensity", 0.0, 3.0, "×"), m("sky.tunnel.rush", 0.0, 12.0, ""),
m("sky.tunnel.twist", -6.0, 6.0, " turns"), m("sky.tunnel.spin", -6.0, 6.0, " turns"), m("sky.tunnel.density", 0.2, 4.0, "×"),
m("sky.tunnel.core", 0.0, 3.0, ""), m("sky.tunnel.light", 0.0, 2.0, ""),
m("sky.rainbow.intensity", 0.0, 1.5, ""), m("sky.rainbow.x", 0.0, 1.0, ""), m("sky.rainbow.size", 0.3, 2.0, "×"),
m("particles.*.count", 0.0, 2000.0, ""), m("particles.*.size", 0.2, 6.0, "×"), m("particles.*.speed", 0.0, 6.0, "×"),
    m("post.exposure", 0.0, 4.0, "×"), m("post.contrast", 0.3, 2.0, "×"), m("post.saturation", 0.0, 2.0, "×"),
    m("post.bloom", 0.0, 2.0, ""), m("post.vignette", 0.0, 1.0, ""), m("post.grain", 0.0, 1.0, ""),
    m("post.focus.distance", 0.5, 60.0, " m"), m("post.focus.blur", 0.0, 0.025, ""), m("post.focus.band", 0.0, 1.0, ""), m("post.focus.band_width", 0.0, 0.8, ""),
    m("style.pixel_size", 1.0, 12.0, " px"), m("style.dither_strength", 0.0, 1.0, ""),
    m("style.paint", 0.0, 8.0, " px"), m("style.grade_strength", 0.0, 1.0, ""), m("style.paper", 0.0, 1.0, ""), m("style.scanlines", 0.0, 1.0, ""),
    m("motion.loop_length", 4.0, 200.0, " m"), m("motion.speed", 0.2, 30.0, " m/s"), m("motion.fps", 6.0, 60.0, " fps"),
    a("*.seed", 0.0, 9999.0, ""), a("*.sprite", 0.0, 0.0, ""),
];

/// Fields never shown (bookkeeping).
pub const HIDDEN: &[&str] = &["version", "name"];

pub fn pattern_of(path: &str) -> String {
    path.split('.').map(|s| if s.parse::<usize>().is_ok() { "*" } else { s }).collect::<Vec<_>>().join(".")
}

pub fn meta(path: &str) -> Option<&'static Meta> {
    let p = pattern_of(path);
    META.iter().find(|m| m.pat == p).or_else(|| META.iter().find(|m| m.pat.starts_with("*.") && p.ends_with(&m.pat[1..])))
}

/// The scene's JSON Schema, indexed for lookups.
pub struct Schema {
    defs: Map<String, Value>,
    root: Value,
}

impl Schema {
    pub fn new() -> Schema {
        let full = serde_json::to_value(schemars::schema_for!(crate::scene::Scene)).unwrap_or(Value::Null);
        let defs = full.get("$defs").and_then(|d| d.as_object()).cloned().unwrap_or_default();
        Schema { defs, root: full }
    }

    pub fn ref_name(s: &Value) -> Option<&str> {
        if let Some(r) = s.get("$ref").and_then(|r| r.as_str()) { return r.rsplit('/').next(); }
        for key in ["allOf", "anyOf"] {
            if let Some(list) = s.get(key).and_then(|l| l.as_array()) {
                for x in list { if let Some(n) = Self::ref_name(x) { return Some(n); } }
            }
        }
        None
    }

    pub fn resolve<'a>(&'a self, s: &'a Value) -> &'a Value {
        Self::ref_name(s).and_then(|n| self.defs.get(n)).unwrap_or(s)
    }

    /// The schema of a field inside a struct schema.
    pub fn field<'a>(&'a self, def: &'a Value, key: &str) -> Option<&'a Value> {
        self.resolve(def).get("properties").and_then(|p| p.get(key))
    }

    /// The schema of a section of the scene (camera, fixtures, ...).
    pub fn section(&self, key: &str) -> Option<&Value> { self.field(&self.root, key) }

    /// The item schema of a list field.
    pub fn items<'a>(&'a self, list: &'a Value) -> Option<&'a Value> { self.resolve(list).get("items") }

    pub fn enum_values(&self, s: &Value) -> Option<Vec<String>> {
        let d = self.resolve(s);
        if let Some(e) = d.get("enum").and_then(|e| e.as_array()) {
            return Some(e.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
        }
        let alts = d.get("oneOf").and_then(|o| o.as_array())?;
        let mut out = Vec::new();
        for alt in alts {
            if let Some(c) = alt.get("const").and_then(|c| c.as_str()) { out.push(c.to_owned()); }
            else if let Some(e) = alt.get("enum").and_then(|e| e.as_array()) { out.extend(e.iter().filter_map(|v| v.as_str().map(str::to_owned))); }
            else { return None; }
        }
        Some(out)
    }

    pub fn description(&self, s: &Value) -> Option<String> {
        s.get("description").or_else(|| self.resolve(s).get("description")).and_then(|d| d.as_str()).map(str::to_owned)
    }
}

/// The scene's parts as an editor shows them, in order: (key, label, item name for a list).
pub const PARTS: &[(&str, &str, Option<&str>)] = &[
    ("sky", "Sky", None), ("path", "Path", None), ("verge", "Sides", None), ("walls", "Walls", None), ("ceiling", "Ceiling", None),
    ("light", "Light", None), ("weather", "Weather", None), ("props", "Props", Some("prop layer")), ("fixtures", "Lights", Some("light")),
    ("set_pieces", "Set pieces", Some("set piece")), ("particles", "Particles", Some("particles")), ("ground_cover", "Ground cover", Some("ground cover")),
    ("companions", "Companions", Some("companion")), ("camera", "Camera", None), ("motion", "Motion", None), ("style", "Style", None),
    ("post", "Post", None), ("canvas", "Canvas", None),
];

/// Every part of the scene as an editor builds itself from it: a list of parts, each a "group"
/// (a struct's fields) or a "list" (of groups). A field is one of
/// - `bool`; `number` (`int`, and `min`/`max`/`unit` where there is a sensible slider);
/// - `enum` (`choices`: value, label, help); `set` (several of `choices`, as a list);
/// - `color` (sRGB [r, g, b]); `colors` (`count` of them); `pair` (x, y in 0..1);
/// - `group` (`fields`); `list` (`item`: a group, `new`: a new item, `one`: what an item is called).
/// Each has `key` (its name in the scene's JSON), `label`, `help` (the doc comment), `default`
/// (what a scene without it means) and `adv` (shown only with advanced settings). Fields an editor
/// on a phone cannot do much with (file paths, a scene's own prop definitions) are left out.
pub fn form() -> Value {
    let schema = Schema::new();
    let parts: Vec<Value> = PARTS.iter().filter_map(|(key, label, one)| {
        let fs = schema.section(key)?;
        let mut v = node(&schema, fs, key, key, 0)?;
        let o = v.as_object_mut()?;
        o.insert("label".into(), json!(label));
        if let Some(one) = one { o.insert("one".into(), json!(one)); }
        if *key == "canvas" || *key == "post" { o.insert("adv".into(), json!(true)); }
        Some(v)
    }).collect();
    json!({ "parts": parts })
}

/// A field's description at `path` (keys joined by dots, `*` for a list's items), or None when an
/// editor should leave it out.
fn node(schema: &Schema, fs: &Value, key: &str, path: &str, depth: usize) -> Option<Value> {
    if HIDDEN.contains(&path) || depth > 8 { return None; }
    let d = schema.resolve(fs);
    let mut o = Map::new();
    o.insert("key".into(), json!(key));
    o.insert("label".into(), json!(label_of(key)));
    if let Some(h) = schema.description(fs) { o.insert("help".into(), json!(h)); }
    if let Some(def) = fs.get("default").or_else(|| d.get("default")) { o.insert("default".into(), def.clone()); }
    let m = meta(path);
    if m.is_some_and(|m| m.adv) { o.insert("adv".into(), json!(true)); }
    let ty = d.get("type").and_then(|t| t.as_str());
    let kind = if let Some(choices) = enum_choices(schema, fs) {
        o.insert("choices".into(), json!(choices));
        "enum"
    } else if d.get("properties").is_some() {
        let props = d["properties"].as_object()?;
        let fields: Vec<Value> = props.iter().filter_map(|(k, f)| node(schema, f, k, &format!("{path}.{k}"), depth + 1)).collect();
        if fields.is_empty() { return None; }
        o.insert("fields".into(), json!(fields));
        "group"
    } else {
        match ty {
            Some("boolean") => "bool",
            Some("integer") | Some("number") => {
                o.insert("int".into(), json!(ty == Some("integer")));
                match m.filter(|m| m.max > m.min) {
                    Some(m) => { o.insert("min".into(), json!(m.min)); o.insert("max".into(), json!(m.max)); o.insert("unit".into(), json!(m.unit.trim())); }
                    None => if let Some(lo) = d.get("minimum") { o.insert("min".into(), lo.clone()); },
                }
                "number"
            }
            Some("array") => {
                let items = d.get("items")?;
                let it = schema.resolve(items);
                let fixed = d.get("minItems").and_then(|n| n.as_u64()).filter(|n| Some(*n) == d.get("maxItems").and_then(|m| m.as_u64()));
                let it_ty = it.get("type").and_then(|t| t.as_str());
                if fixed == Some(3) && it_ty == Some("integer") { "color" }
                else if fixed == Some(2) && it_ty == Some("number") { "pair" }
                else if let (Some(n), Some("array")) = (fixed, it_ty) {
                    let inner = it.get("items").map(|x| schema.resolve(x));
                    if inner.and_then(|x| x.get("type")).and_then(|t| t.as_str()) != Some("integer") { return None; }
                    o.insert("count".into(), json!(n));
                    "colors"
                } else if let Some(choices) = enum_choices(schema, items) {
                    o.insert("choices".into(), json!(choices));
                    "set"
                } else if it.get("properties").is_some() {
                    let item = node(schema, items, "*", &format!("{path}.*"), depth + 1)?;
                    o.insert("new".into(), new_item(schema, items));
                    o.insert("item".into(), item);
                    o.insert("one".into(), json!(label_of(key).trim_end_matches('s').to_lowercase()));
                    "list"
                } else { return None; }
            }
            _ => return None,
        }
    };
    o.insert("kind".into(), json!(kind));
    Some(Value::Object(o))
}

/// An enum's choices (value, label, help), or None when it is not a plain choice of names.
fn enum_choices(schema: &Schema, fs: &Value) -> Option<Vec<Value>> {
    let d = schema.resolve(fs);
    if let Some(e) = d.get("enum").and_then(|e| e.as_array()) {
        return Some(e.iter().filter_map(|v| v.as_str()).map(|v| json!({ "value": v, "label": spaced(v) })).collect());
    }
    let alts = d.get("oneOf").and_then(|o| o.as_array())?;
    let mut out = Vec::new();
    for alt in alts {
        let help = alt.get("description").cloned().unwrap_or(Value::Null);
        if let Some(c) = alt.get("const").and_then(|c| c.as_str()) { out.push(json!({ "value": c, "label": spaced(c), "help": help })); }
        else if let Some(e) = alt.get("enum").and_then(|e| e.as_array()) {
            out.extend(e.iter().filter_map(|v| v.as_str()).map(|v| json!({ "value": v, "label": spaced(v), "help": help })));
        } else { return None; }
    }
    Some(out)
}

/// A new item for a list of structs: every field at its default, switched on.
pub fn new_item(schema: &Schema, items: &Value) -> Value {
    let def = schema.resolve(items);
    let mut o = Map::new();
    if let Some(props) = def.get("properties").and_then(|p| p.as_object()) {
        for (k, fs) in props {
            if let Some(d) = fs.get("default").or_else(|| schema.resolve(fs).get("default")) { o.insert(k.clone(), d.clone()); }
        }
    }
    if o.contains_key("enabled") { o.insert("enabled".into(), json!(true)); }
    Value::Object(o)
}

/// "tile_size" → "Tile size".
pub fn label_of(key: &str) -> String {
    let s = key.replace('_', " ");
    let mut c = s.chars();
    match c.next() { Some(f) => f.to_uppercase().collect::<String>() + c.as_str(), None => s }
}

/// "LilyPads" → "Lily Pads".
pub fn spaced(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && ch.is_uppercase() { out.push(' '); }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(fields: &'a Value, key: &str) -> &'a Value {
        fields["fields"].as_array().unwrap().iter().find(|f| f["key"] == key).unwrap_or_else(|| panic!("no {key}"))
    }

    #[test]
    fn the_form_covers_the_scene() {
        let f = form();
        let parts = f["parts"].as_array().unwrap();
        let part = |k: &str| parts.iter().find(|p| p["key"] == k).unwrap_or_else(|| panic!("no part {k}"));
        // A slider with its range, a colour, an enum with its help, a list of groups, a set.
        let sky = part("sky");
        let sunset = find(sky, "sunset");
        assert_eq!(find(sunset, "amount")["max"], 1.0);
        assert_eq!(find(sunset, "style")["kind"], "enum");
        assert!(find(sunset, "style")["choices"].as_array().unwrap().iter().any(|c| c["value"] == "Fiery" && c["help"].is_string()));
        assert_eq!(find(sky, "top")["kind"], "color");
        assert_eq!(find(find(sky, "sun"), "pos")["kind"], "pair");
        let layers = find(sky, "cloud_layers");
        assert_eq!(layers["kind"], "list");
        assert_eq!(layers["new"]["enabled"], true);
        assert_eq!(find(find(sky, "cloud_mix"), "kinds")["kind"], "set");
        assert_eq!(part("props")["kind"], "list");
        assert_eq!(part("props")["one"], "prop layer");
        // Seeds are advanced; file paths are left out.
        assert_eq!(find(find(sky, "clouds"), "seed")["adv"], true);
        assert!(part("props")["item"]["fields"].as_array().unwrap().iter().all(|f| f["key"] != "def"));
        // Every number has a slider range or at least a floor, and the whole form is small enough
        // to hand to a phone.
        assert!(serde_json::to_string(&f).unwrap().len() < 2_000_000);
    }
}
