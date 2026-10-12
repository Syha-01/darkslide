//! The develop settings model. Mirrors Camera Raw's `crs:` settings; slider
//! values use the same units and ranges as the ACR UI.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevelopSettings {
    pub process_version: String,

    // Profile
    pub profile: String,
    pub profile_amount: f32,
    pub look: Option<Look>,
    pub treatment: Treatment,

    // White balance
    pub wb_mode: WbMode,
    /// Kelvin for raw files, -100..100 for rendered files.
    pub temperature: f32,
    pub tint: f32,

    // Basic
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub hdr: HdrSettings,

    // Curve
    pub parametric: ParametricCurve,
    pub curve: PointCurves,
    pub refine_saturation: f32,

    // Detail
    pub sharpen: Sharpening,
    pub nr: NoiseReduction,
    pub enhance: Enhance,

    // Color mixer
    pub hsl: [Hsl; 8],
    pub gray_mix: [f32; 8],
    pub auto_gray_mix: bool,

    pub point_colors: Vec<PointColor>,
    pub grading: ColorGrading,

    // Optics
    pub lens: LensCorrection,
    pub lens_blur: LensBlur,
    pub distractions: DistractionRemoval,

    // Geometry
    pub transform: Transform,
    pub crop: Crop,

    // Effects
    pub vignette: PostCropVignette,
    pub grain: Grain,

    pub calibration: Calibration,

    // Local
    pub masks: Vec<MaskGroup>,
    pub retouch: Vec<RetouchSpot>,
    pub red_eye: Vec<RedEyeSpot>,

    pub panels: PanelToggles,
}

pub const HSL_BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

impl Default for DevelopSettings {
    fn default() -> Self {
        Self::defaults_for(true)
    }
}

impl DevelopSettings {
    pub fn defaults_for(is_raw: bool) -> Self {
        Self {
            process_version: "11.0".into(),
            profile: "Color".into(),
            profile_amount: 100.0,
            look: None,
            treatment: Treatment::Color,
            wb_mode: WbMode::AsShot,
            temperature: if is_raw { 5500.0 } else { 0.0 },
            tint: if is_raw { 10.0 } else { 0.0 },
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            hdr: HdrSettings::default(),
            parametric: ParametricCurve::default(),
            curve: PointCurves::default(),
            refine_saturation: 100.0,
            sharpen: Sharpening { amount: if is_raw { 40.0 } else { 0.0 }, ..Default::default() },
            nr: NoiseReduction { color: if is_raw { 25.0 } else { 0.0 }, ..Default::default() },
            enhance: Enhance::default(),
            hsl: [Hsl::default(); 8],
            gray_mix: [0.0; 8],
            auto_gray_mix: true,
            point_colors: vec![],
            grading: ColorGrading::default(),
            lens: LensCorrection::default(),
            lens_blur: LensBlur::default(),
            distractions: DistractionRemoval::default(),
            transform: Transform::default(),
            crop: Crop::default(),
            vignette: PostCropVignette::default(),
            grain: Grain::default(),
            calibration: Calibration::default(),
            masks: vec![],
            retouch: vec![],
            red_eye: vec![],
            panels: PanelToggles::default(),
        }
    }

    pub fn is_bw(&self) -> bool {
        self.treatment == Treatment::BlackAndWhite
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Treatment {
    #[default]
    Color,
    BlackAndWhite,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Look {
    pub name: String,
    /// Path to a .cube LUT or a built-in look id ("builtin:<name>").
    pub source: String,
    pub amount: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WbMode {
    #[default]
    AsShot,
    Auto,
    Daylight,
    Cloudy,
    Shade,
    Tungsten,
    Fluorescent,
    Flash,
    Custom,
}

impl WbMode {
    pub const ALL: [WbMode; 9] =
        [Self::AsShot, Self::Auto, Self::Daylight, Self::Cloudy, Self::Shade, Self::Tungsten, Self::Fluorescent, Self::Flash, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::AsShot => "As Shot",
            Self::Auto => "Auto",
            Self::Daylight => "Daylight",
            Self::Cloudy => "Cloudy",
            Self::Shade => "Shade",
            Self::Tungsten => "Tungsten",
            Self::Fluorescent => "Fluorescent",
            Self::Flash => "Flash",
            Self::Custom => "Custom",
        }
    }

    pub fn crs_name(self) -> &'static str {
        match self {
            Self::AsShot => "As Shot",
            other => other.label(),
        }
    }

    pub fn from_crs(s: &str) -> Self {
        Self::ALL.iter().copied().find(|m| m.crs_name() == s).unwrap_or(Self::Custom)
    }

    /// Preset (temperature, tint) for fixed presets, as ACR uses.
    pub fn preset(self) -> Option<(f32, f32)> {
        match self {
            Self::Daylight => Some((5500.0, 10.0)),
            Self::Cloudy => Some((6500.0, 10.0)),
            Self::Shade => Some((7500.0, 10.0)),
            Self::Tungsten => Some((2850.0, 0.0)),
            Self::Fluorescent => Some((3800.0, 21.0)),
            Self::Flash => Some((5500.0, 0.0)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HdrSettings {
    pub enabled: bool,
    /// Visualise HDR range overlay in the preview.
    pub visualize: bool,
    pub sdr_brightness: f32,
    pub sdr_contrast: f32,
    pub sdr_clarity: f32,
    pub sdr_highlights: f32,
    pub sdr_shadows: f32,
    pub sdr_whites: f32,
    pub sdr_blend: f32,
    /// Max headroom in stops above SDR white.
    pub max_headroom: f32,
}

impl Default for HdrSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            visualize: false,
            sdr_brightness: 0.0,
            sdr_contrast: 0.0,
            sdr_clarity: 0.0,
            sdr_highlights: 0.0,
            sdr_shadows: 0.0,
            sdr_whites: 0.0,
            sdr_blend: 0.0,
            max_headroom: 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParametricCurve {
    pub highlights: f32,
    pub lights: f32,
    pub darks: f32,
    pub shadows: f32,
    pub shadow_split: f32,
    pub midtone_split: f32,
    pub highlight_split: f32,
}

impl Default for ParametricCurve {
    fn default() -> Self {
        Self { highlights: 0.0, lights: 0.0, darks: 0.0, shadows: 0.0, shadow_split: 25.0, midtone_split: 50.0, highlight_split: 75.0 }
    }
}

impl ParametricCurve {
    pub fn is_identity(&self) -> bool {
        self.highlights == 0.0 && self.lights == 0.0 && self.darks == 0.0 && self.shadows == 0.0
    }
}

/// Point curve control points in 0..255 (ACR convention).
pub type CurvePoints = Vec<[f32; 2]>;

pub fn linear_curve() -> CurvePoints {
    vec![[0.0, 0.0], [255.0, 255.0]]
}

pub fn is_linear_curve(c: &CurvePoints) -> bool {
    c.iter().all(|p| (p[0] - p[1]).abs() < 1e-3)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointCurves {
    pub rgb: CurvePoints,
    pub red: CurvePoints,
    pub green: CurvePoints,
    pub blue: CurvePoints,
}

impl Default for PointCurves {
    fn default() -> Self {
        Self { rgb: linear_curve(), red: linear_curve(), green: linear_curve(), blue: linear_curve() }
    }
}

impl PointCurves {
    pub fn is_identity(&self) -> bool {
        is_linear_curve(&self.rgb) && is_linear_curve(&self.red) && is_linear_curve(&self.green) && is_linear_curve(&self.blue)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sharpening {
    pub amount: f32,
    pub radius: f32,
    pub detail: f32,
    pub masking: f32,
}

impl Default for Sharpening {
    fn default() -> Self {
        Self { amount: 40.0, radius: 1.0, detail: 25.0, masking: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoiseReduction {
    pub luminance: f32,
    pub luminance_detail: f32,
    pub luminance_contrast: f32,
    pub color: f32,
    pub color_detail: f32,
    pub color_smoothness: f32,
}

impl Default for NoiseReduction {
    fn default() -> Self {
        Self { luminance: 0.0, luminance_detail: 50.0, luminance_contrast: 0.0, color: 25.0, color_detail: 50.0, color_smoothness: 50.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Enhance {
    pub denoise: bool,
    pub denoise_amount: f32,
    pub raw_details: bool,
    pub super_resolution: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Hsl {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointColor {
    /// Sampled colour as Oklch (L 0..1, C, h degrees).
    pub sample: [f32; 3],
    pub hue_shift: f32,
    pub sat_shift: f32,
    pub lum_shift: f32,
    /// Overall range 0..100 and per-axis ranges.
    pub range: f32,
    pub hue_range: f32,
    pub sat_range: f32,
    pub lum_range: f32,
    /// Colour variance -100..100: compresses (negative) or expands hue spread.
    pub variance: f32,
}

impl Default for PointColor {
    fn default() -> Self {
        Self {
            sample: [0.6, 0.1, 30.0],
            hue_shift: 0.0,
            sat_shift: 0.0,
            lum_shift: 0.0,
            range: 50.0,
            hue_range: 50.0,
            sat_range: 50.0,
            lum_range: 50.0,
            variance: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Wheel {
    pub hue: f32,
    pub sat: f32,
    pub lum: f32,
}

impl Wheel {
    pub fn is_identity(&self) -> bool {
        self.sat == 0.0 && self.lum == 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    pub shadows: Wheel,
    pub midtones: Wheel,
    pub highlights: Wheel,
    pub global: Wheel,
    pub blending: f32,
    pub balance: f32,
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self { shadows: Wheel::default(), midtones: Wheel::default(), highlights: Wheel::default(), global: Wheel::default(), blending: 50.0, balance: 0.0 }
    }
}

impl ColorGrading {
    pub fn is_identity(&self) -> bool {
        self.shadows.is_identity() && self.midtones.is_identity() && self.highlights.is_identity() && self.global.is_identity()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LensCorrection {
    pub remove_ca: bool,
    pub profile_enable: bool,
    /// Lens profile identifier (lensfun "maker|model"), None = auto.
    pub profile: Option<String>,
    pub distortion_scale: f32,
    pub vignette_scale: f32,
    pub manual_distortion: f32,
    pub vignette_amount: f32,
    pub vignette_midpoint: f32,
    pub defringe_purple_amount: f32,
    pub defringe_purple_hue: [f32; 2],
    pub defringe_green_amount: f32,
    pub defringe_green_hue: [f32; 2],
}

impl Default for LensCorrection {
    fn default() -> Self {
        Self {
            remove_ca: false,
            profile_enable: false,
            profile: None,
            distortion_scale: 100.0,
            vignette_scale: 100.0,
            manual_distortion: 0.0,
            vignette_amount: 0.0,
            vignette_midpoint: 50.0,
            defringe_purple_amount: 0.0,
            defringe_purple_hue: [30.0, 70.0],
            defringe_green_amount: 0.0,
            defringe_green_hue: [40.0, 60.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BokehShape {
    #[default]
    Circle,
    Bubble,
    FiveBlade,
    Ring,
    CatEye,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LensBlur {
    pub enabled: bool,
    pub amount: f32,
    pub shape: BokehShape,
    /// Focal range in normalised depth [near, far].
    pub focal_range: [f32; 2],
    pub boost: f32,
    pub visualize_depth: bool,
}

impl Default for LensBlur {
    fn default() -> Self {
        Self { enabled: false, amount: 50.0, shape: BokehShape::Circle, focal_range: [0.0, 0.2], boost: 50.0, visualize_depth: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DistractionRemoval {
    pub reflections: f32,
    pub dust: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Upright {
    #[default]
    Off,
    Auto,
    Level,
    Vertical,
    Full,
    Guided,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub upright: Upright,
    /// Guided upright lines in normalised image coords: [x0,y0,x1,y1].
    pub guides: Vec<[f32; 4]>,
    pub vertical: f32,
    pub horizontal: f32,
    pub rotate: f32,
    pub aspect: f32,
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub constrain_crop: bool,
    /// Homography computed by Upright analysis (cached in settings so the
    /// render is deterministic). Row-major 3x3 in normalised coords.
    pub upright_matrix: Option<[f32; 9]>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            upright: Upright::Off,
            guides: vec![],
            vertical: 0.0,
            horizontal: 0.0,
            rotate: 0.0,
            aspect: 0.0,
            scale: 100.0,
            offset_x: 0.0,
            offset_y: 0.0,
            constrain_crop: false,
            upright_matrix: None,
        }
    }
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        self.upright_matrix.is_none()
            && self.vertical == 0.0
            && self.horizontal == 0.0
            && self.rotate == 0.0
            && self.aspect == 0.0
            && self.scale == 100.0
            && self.offset_x == 0.0
            && self.offset_y == 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crop {
    /// Normalised crop rectangle in the oriented, transformed frame (before
    /// straighten rotation is applied around its center).
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// Straighten angle in degrees (-45..45).
    pub angle: f32,
    /// Extra user rotation in 90° steps (0..3, clockwise).
    pub rotate90: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Aspect ratio lock (w, h); None = free.
    pub aspect: Option<[f32; 2]>,
    /// Allow the crop to extend past the image (Generative Expand area).
    pub expand: bool,
}

impl Default for Crop {
    fn default() -> Self {
        Self { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0, angle: 0.0, rotate90: 0, flip_h: false, flip_v: false, aspect: None, expand: false }
    }
}

impl Crop {
    pub fn has_crop(&self) -> bool {
        self.left != 0.0 || self.top != 0.0 || self.right != 1.0 || self.bottom != 1.0 || self.angle != 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VignetteStyle {
    #[default]
    HighlightPriority,
    ColorPriority,
    PaintOverlay,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostCropVignette {
    pub style: VignetteStyle,
    pub amount: f32,
    pub midpoint: f32,
    pub roundness: f32,
    pub feather: f32,
    pub highlights: f32,
}

impl Default for PostCropVignette {
    fn default() -> Self {
        Self { style: VignetteStyle::HighlightPriority, amount: 0.0, midpoint: 50.0, roundness: 0.0, feather: 50.0, highlights: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grain {
    pub amount: f32,
    pub size: f32,
    pub roughness: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self { amount: 0.0, size: 25.0, roughness: 50.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Calibration {
    pub shadow_tint: f32,
    pub red_hue: f32,
    pub red_sat: f32,
    pub green_hue: f32,
    pub green_sat: f32,
    pub blue_hue: f32,
    pub blue_sat: f32,
}

impl Calibration {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelToggles {
    pub basic: bool,
    pub curve: bool,
    pub detail: bool,
    pub mixer: bool,
    pub point_color: bool,
    pub grading: bool,
    pub optics: bool,
    pub geometry: bool,
    pub effects: bool,
    pub calibration: bool,
}

impl Default for PanelToggles {
    fn default() -> Self {
        Self {
            basic: true,
            curve: true,
            detail: true,
            mixer: true,
            point_color: true,
            grading: true,
            optics: true,
            geometry: true,
            effects: true,
            calibration: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Masking

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskGroup {
    pub name: String,
    pub enabled: bool,
    pub invert: bool,
    /// Scales every local adjustment, 0..200.
    pub amount: f32,
    pub components: Vec<MaskComponent>,
    pub adjust: LocalAdjust,
}

impl Default for MaskGroup {
    fn default() -> Self {
        Self { name: "Mask 1".into(), enabled: true, invert: false, amount: 100.0, components: vec![], adjust: LocalAdjust::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaskOp {
    #[default]
    Add,
    Subtract,
    Intersect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskComponent {
    pub op: MaskOp,
    pub invert: bool,
    pub opacity: f32,
    pub shape: MaskShape,
}

impl Default for MaskComponent {
    fn default() -> Self {
        Self { op: MaskOp::Add, invert: false, opacity: 100.0, shape: MaskShape::Brush { strokes: vec![] } }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushStroke {
    /// Points in normalised coords (x/w, y/h).
    pub points: Vec<[f32; 2]>,
    /// Radius as a fraction of the long edge.
    pub radius: f32,
    pub feather: f32,
    pub flow: f32,
    pub density: f32,
    pub erase: bool,
    pub auto_mask: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiMaskKind {
    Subject,
    Sky,
    Background,
    Person,
    Object,
    Water,
    Vegetation,
    Mountains,
    Architecture,
    NaturalGround,
    ArtificialGround,
    Snow,
}

impl AiMaskKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Subject => "Subject",
            Self::Sky => "Sky",
            Self::Background => "Background",
            Self::Person => "People",
            Self::Object => "Objects",
            Self::Water => "Water",
            Self::Vegetation => "Vegetation",
            Self::Mountains => "Mountains",
            Self::Architecture => "Architecture",
            Self::NaturalGround => "Natural ground",
            Self::ArtificialGround => "Artificial ground",
            Self::Snow => "Snow",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MaskShape {
    Brush {
        strokes: Vec<BrushStroke>,
    },
    /// Gradient from full effect at `start` to none at `end` (normalised coords).
    Linear {
        start: [f32; 2],
        end: [f32; 2],
        two_sided: bool,
    },
    /// Ellipse center, radii (normalised to width/height), rotation degrees,
    /// feather 0..100.
    Radial {
        center: [f32; 2],
        radius: [f32; 2],
        angle: f32,
        feather: f32,
    },
    /// Colour samples (Oklab) with a refine amount 0..100.
    ColorRange {
        samples: Vec<[f32; 3]>,
        refine: f32,
    },
    /// Luminance range 0..1 with smoothness 0..100.
    LuminanceRange {
        range: [f32; 2],
        smoothness: f32,
    },
    DepthRange {
        range: [f32; 2],
        smoothness: f32,
    },
    /// AI / detection-based mask. `raster` caches the computed alpha
    /// (base64 8-bit PNG) so renders are deterministic.
    Ai {
        kind: AiMaskKind,
        feather: f32,
        edge: f32,
        hint: Option<[f32; 4]>,
        raster: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAdjust {
    pub temperature: f32,
    pub tint: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub hue: f32,
    pub saturation: f32,
    pub sharpness: f32,
    pub noise: f32,
    pub moire: f32,
    pub defringe: f32,
    /// Tint colour overlay (hue degrees, saturation 0..100).
    pub color: [f32; 2],
    pub grading: ColorGrading,
    pub curve: PointCurves,
    pub point_colors: Vec<PointColor>,
}

impl Default for LocalAdjust {
    fn default() -> Self {
        Self {
            temperature: 0.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            hue: 0.0,
            saturation: 0.0,
            sharpness: 0.0,
            noise: 0.0,
            moire: 0.0,
            defringe: 0.0,
            color: [0.0, 0.0],
            grading: ColorGrading::default(),
            curve: PointCurves::default(),
            point_colors: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Retouch

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RetouchMode {
    #[default]
    Heal,
    Clone,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetouchSpot {
    pub mode: RetouchMode,
    /// Destination stroke (a single point = circular spot). Normalised coords.
    pub points: Vec<[f32; 2]>,
    /// Radius as a fraction of the long edge.
    pub radius: f32,
    pub feather: f32,
    pub opacity: f32,
    /// Source offset in normalised coords (heal/clone).
    pub source_offset: [f32; 2],
    /// Detected region (base64 PNG raster, S frame) for distraction removal;
    /// used instead of the stroke when present.
    #[serde(default)]
    pub mask: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedEyeSpot {
    pub center: [f32; 2],
    pub radius: [f32; 2],
    pub pupil_size: f32,
    pub darken: f32,
    pub pet: bool,
    pub catchlight: bool,
}
