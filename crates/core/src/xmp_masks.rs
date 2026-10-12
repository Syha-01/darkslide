//! Camera Raw / Lightroom local corrections (`crs:MaskGroupBasedCorrections`)
//! <-> our mask groups. Linear, radial and brush masks with the scalar local
//! sliders map both ways; other mask kinds stay in our JSON copy only.

use quick_xml::events::Event;

use crate::params::*;

#[derive(Debug, Default, Clone)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn attr(&self, k: &str) -> Option<&str> {
        // properties may be attributes or child elements with text
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).or_else(|| self.children.iter().find(|c| c.name == k).map(|c| c.text.as_str()))
    }
    pub fn f(&self, k: &str) -> Option<f32> {
        self.attr(k).and_then(|v| v.trim().trim_start_matches('+').parse().ok())
    }
    pub fn child(&self, k: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == k)
    }
    /// Items of an rdf:Seq/Bag under this node; each item normalised to the
    /// node carrying the properties (rdf:li or its rdf:Description).
    pub fn items(&self) -> Vec<&Node> {
        let seq = self.children.iter().find(|c| c.name == "rdf:Seq" || c.name == "rdf:Bag");
        seq.map(|s| s.children.iter().filter(|c| c.name == "rdf:li").map(|li| li.child("rdf:Description").unwrap_or(li)).collect()).unwrap_or_default()
    }
    pub fn find(&self, name: &str) -> Option<&Node> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }
}

pub fn parse_dom(xml: &str) -> Node {
    let mut r = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<Node> = vec![Node { name: "#root".into(), ..Default::default() }];
    let mut buf = vec![];
    loop {
        match r.read_event_into(&mut buf) {
            Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                let empty = matches!(ev, Event::Empty(_));
                let e = match ev {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let mut n = Node { name: String::from_utf8_lossy(e.name().as_ref()).to_string(), ..Default::default() };
                for a in e.attributes().flatten() {
                    n.attrs.push((String::from_utf8_lossy(a.key.as_ref()).to_string(), a.unescape_value().map(|v| v.to_string()).unwrap_or_default()));
                }
                if empty {
                    stack.last_mut().unwrap().children.push(n);
                } else {
                    stack.push(n);
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.decode().map(|c| c.to_string()).unwrap_or_default());
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let n = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(n);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    while stack.len() > 1 {
        let n = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(n);
    }
    stack.pop().unwrap()
}

/// (crs local name, slider scale: ACR stores -1..1 for ±100, EV for exposure)
const LOCAL_FIELDS: &[(&str, f32)] = &[
    ("LocalExposure2012", 1.0),
    ("LocalContrast2012", 100.0),
    ("LocalHighlights2012", 100.0),
    ("LocalShadows2012", 100.0),
    ("LocalWhites2012", 100.0),
    ("LocalBlacks2012", 100.0),
    ("LocalTexture", 100.0),
    ("LocalClarity2012", 100.0),
    ("LocalDehaze", 100.0),
    ("LocalSaturation", 100.0),
    ("LocalTemperature", 100.0),
    ("LocalTint", 100.0),
    ("LocalHue", 100.0),
    ("LocalSharpness", 100.0),
    ("LocalLuminanceNoise", 100.0),
    ("LocalMoire", 100.0),
    ("LocalDefringe", 100.0),
];

fn field<'a>(a: &'a mut LocalAdjust, name: &str) -> Option<&'a mut f32> {
    Some(match name {
        "LocalExposure2012" => &mut a.exposure,
        "LocalContrast2012" => &mut a.contrast,
        "LocalHighlights2012" => &mut a.highlights,
        "LocalShadows2012" => &mut a.shadows,
        "LocalWhites2012" => &mut a.whites,
        "LocalBlacks2012" => &mut a.blacks,
        "LocalTexture" => &mut a.texture,
        "LocalClarity2012" => &mut a.clarity,
        "LocalDehaze" => &mut a.dehaze,
        "LocalSaturation" => &mut a.saturation,
        "LocalTemperature" => &mut a.temperature,
        "LocalTint" => &mut a.tint,
        "LocalHue" => &mut a.hue,
        "LocalSharpness" => &mut a.sharpness,
        "LocalLuminanceNoise" => &mut a.noise,
        "LocalMoire" => &mut a.moire,
        "LocalDefringe" => &mut a.defringe,
        _ => return None,
    })
}

fn component_from(n: &Node) -> Option<MaskComponent> {
    let what = n.attr("crs:What")?;
    let op = match n.attr("crs:MaskBlendMode").unwrap_or("0") {
        "1" => MaskOp::Intersect,
        "2" => MaskOp::Subtract,
        _ => MaskOp::Add,
    };
    let invert = n.attr("crs:MaskInverted").map(|v| v.eq_ignore_ascii_case("true")).unwrap_or(false);
    let opacity = n.f("crs:MaskValue").unwrap_or(1.0) * 100.0;
    let shape = match what {
        "Mask/Gradient" => {
            let start = [n.f("crs:FullX")?, n.f("crs:FullY")?];
            let end = [n.f("crs:ZeroX")?, n.f("crs:ZeroY")?];
            MaskShape::Linear { start, end, two_sided: false }
        }
        "Mask/CircularGradient" => {
            let (t, l, b, r) = (n.f("crs:Top")?, n.f("crs:Left")?, n.f("crs:Bottom")?, n.f("crs:Right")?);
            MaskShape::Radial {
                center: [(l + r) / 2.0, (t + b) / 2.0],
                radius: [((r - l) / 2.0).abs(), ((b - t) / 2.0).abs()],
                angle: n.f("crs:Angle").unwrap_or(0.0),
                feather: n.f("crs:Feather").unwrap_or(50.0),
            }
        }
        "Mask/Paint" => {
            let radius = n.f("crs:Radius").unwrap_or(0.05);
            let flow = n.f("crs:Flow").unwrap_or(1.0) * 100.0;
            let feather = n.f("crs:CenterWeight").map(|c| (1.0 - c) * 100.0).unwrap_or(50.0);
            let erase = n.attr("crs:MaskValue").map(|v| v.trim() == "0").unwrap_or(false);
            let points: Vec<[f32; 2]> = n
                .child("crs:Dabs")
                .map(|d| {
                    d.items()
                        .iter()
                        .filter_map(|li| {
                            let mut it = li.text.split_whitespace();
                            let _d = it.next()?;
                            Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?])
                        })
                        .collect()
                })
                .unwrap_or_default();
            if points.is_empty() {
                return None;
            }
            MaskShape::Brush { strokes: vec![BrushStroke { points, radius, feather, flow, density: 100.0, erase, auto_mask: false }] }
        }
        _ => return None,
    };
    Some(MaskComponent { op, invert, opacity: if what == "Mask/Paint" { 100.0 } else { opacity }, shape })
}

/// Read ACR mask groups from an XMP packet.
pub fn read(xml: &str) -> Vec<MaskGroup> {
    let dom = parse_dom(xml);
    let Some(root) = dom.find("crs:MaskGroupBasedCorrections") else { return vec![] };
    let mut out = vec![];
    for (i, corr) in root.items().into_iter().enumerate() {
        let mut g =
            MaskGroup { name: corr.attr("crs:CorrectionName").map(|s| s.to_string()).unwrap_or_else(|| format!("Mask {}", i + 1)), ..Default::default() };
        g.enabled = corr.attr("crs:CorrectionActive").map(|v| !v.eq_ignore_ascii_case("false")).unwrap_or(true);
        g.amount = corr.f("crs:CorrectionAmount").unwrap_or(1.0) * 100.0;
        for (name, scale) in LOCAL_FIELDS {
            if let (Some(v), Some(f)) = (corr.f(&format!("crs:{name}")), field(&mut g.adjust, name)) {
                *f = v * scale;
            }
        }
        if let (Some(h), Some(s)) = (corr.f("crs:LocalToningHue"), corr.f("crs:LocalToningSaturation")) {
            g.adjust.color = [h, s * 100.0];
        }
        if let Some(masks) = corr.child("crs:CorrectionMasks") {
            g.components = masks.items().into_iter().filter_map(component_from).collect();
        }
        if !g.components.is_empty() {
            out.push(g);
        }
    }
    out
}

fn esc(s: &str) -> String {
    quick_xml::escape::escape(s).to_string()
}

/// Serialise the mask groups ACR can represent.
pub fn write(masks: &[MaskGroup]) -> String {
    let mut x = String::new();
    let mut any = false;
    let mut body = String::new();
    for g in masks {
        let comps: Vec<String> = g
            .components
            .iter()
            .enumerate()
            .filter_map(|(ci, c)| {
                let mode = match (ci, c.op) {
                    (0, _) | (_, MaskOp::Add) => 0,
                    (_, MaskOp::Intersect) => 1,
                    (_, MaskOp::Subtract) => 2,
                };
                let common = format!(r#"crs:MaskValue="{}" crs:MaskBlendMode="{mode}" crs:MaskInverted="{}""#, c.opacity / 100.0, if c.invert { "true" } else { "false" });
                match &c.shape {
                    MaskShape::Linear { start, end, .. } => Some(format!(
                        r#"<rdf:li crs:What="Mask/Gradient" {common} crs:FullX="{}" crs:FullY="{}" crs:ZeroX="{}" crs:ZeroY="{}"/>"#,
                        start[0], start[1], end[0], end[1]
                    )),
                    MaskShape::Radial { center, radius, angle, feather } => Some(format!(
                        r#"<rdf:li crs:What="Mask/CircularGradient" {common} crs:Top="{}" crs:Left="{}" crs:Bottom="{}" crs:Right="{}" crs:Angle="{angle}" crs:Midpoint="50" crs:Roundness="0" crs:Feather="{feather}" crs:Flipped="false" crs:Version="2"/>"#,
                        center[1] - radius[1],
                        center[0] - radius[0],
                        center[1] + radius[1],
                        center[0] + radius[0]
                    )),
                    MaskShape::Brush { strokes } => {
                        let mut s = String::new();
                        for st in strokes {
                            let dabs: String = st.points.iter().map(|p| format!("<rdf:li>d {} {}</rdf:li>", p[0], p[1])).collect();
                            s.push_str(&format!(
                                r#"<rdf:li crs:What="Mask/Paint" crs:MaskValue="{}" crs:Radius="{}" crs:Flow="{}" crs:CenterWeight="{}"><crs:Dabs><rdf:Seq>{dabs}</rdf:Seq></crs:Dabs></rdf:li>"#,
                                if st.erase { 0.0 } else { st.density / 100.0 },
                                st.radius,
                                st.flow / 100.0,
                                1.0 - st.feather / 100.0
                            ));
                        }
                        Some(s)
                    }
                    _ => None,
                }
            })
            .collect();
        if comps.is_empty() {
            continue;
        }
        any = true;
        let mut a = g.adjust.clone();
        let mut attrs = String::new();
        for (name, scale) in LOCAL_FIELDS {
            if let Some(v) = field(&mut a, name) {
                attrs.push_str(&format!(r#" crs:{name}="{}""#, *v / scale));
            }
        }
        if g.adjust.color[1] > 0.0 {
            attrs.push_str(&format!(r#" crs:LocalToningHue="{}" crs:LocalToningSaturation="{}""#, g.adjust.color[0], g.adjust.color[1] / 100.0));
        }
        body.push_str(&format!(
            r#"<rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="{}" crs:CorrectionActive="{}" crs:CorrectionName="{}"{attrs}><crs:CorrectionMasks><rdf:Seq>{}</rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li>"#,
            g.amount / 100.0,
            if g.enabled { "true" } else { "false" },
            esc(&g.name),
            comps.join("")
        ));
    }
    if any {
        x.push_str("   <crs:MaskGroupBasedCorrections><rdf:Seq>");
        x.push_str(&body);
        x.push_str("</rdf:Seq></crs:MaskGroupBasedCorrections>\n");
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_linear_radial_brush() {
        let masks = vec![MaskGroup {
            name: "Test".into(),
            amount: 80.0,
            components: vec![
                MaskComponent { shape: MaskShape::Linear { start: [0.5, 0.2], end: [0.5, 0.5], two_sided: false }, ..Default::default() },
                MaskComponent {
                    op: MaskOp::Subtract,
                    shape: MaskShape::Radial { center: [0.4, 0.6], radius: [0.1, 0.2], angle: 0.0, feather: 40.0 },
                    ..Default::default()
                },
                MaskComponent {
                    shape: MaskShape::Brush {
                        strokes: vec![BrushStroke {
                            points: vec![[0.1, 0.1], [0.2, 0.15]],
                            radius: 0.03,
                            feather: 50.0,
                            flow: 50.0,
                            density: 100.0,
                            erase: false,
                            auto_mask: false,
                        }],
                    },
                    ..Default::default()
                },
            ],
            adjust: LocalAdjust { exposure: 0.7, clarity: 25.0, ..Default::default() },
            ..Default::default()
        }];
        let xml = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/">{}</rdf:Description></rdf:RDF></x:xmpmeta>"#,
            write(&masks)
        );
        let back = read(&xml);
        assert_eq!(back.len(), 1);
        let g = &back[0];
        assert_eq!(g.components.len(), 3);
        assert!((g.adjust.exposure - 0.7).abs() < 1e-5);
        assert!((g.adjust.clarity - 25.0).abs() < 1e-3);
        assert!((g.amount - 80.0).abs() < 1e-3);
        assert_eq!(g.components[1].op, MaskOp::Subtract);
        match &g.components[1].shape {
            MaskShape::Radial { center, radius, .. } => {
                assert!((center[0] - 0.4).abs() < 1e-5 && (radius[1] - 0.2).abs() < 1e-5);
            }
            _ => panic!("expected radial"),
        }
        match &g.components[2].shape {
            MaskShape::Brush { strokes } => assert_eq!(strokes[0].points.len(), 2),
            _ => panic!("expected brush"),
        }
    }
}
