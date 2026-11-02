// Project window feather formulas and f32 counterpart of windows.rs. Geometry and threaded
// BVH addresses live in the existing parameters tail; no geometry is shader source.
fn window_feather(d: f32, inner: f32, outer: f32) -> f32 {
    if inner + outer == 0.0 { return select(0.0, 1.0, d >= 0.0); }
    return smoothstep(-outer, inner, d);
}

fn window_distance(p: vec2<f32>, e: vec4<f32>) -> f32 {
    let d = e.zw - e.xy;
    let n = dot(d, d);
    var t = 0.0;
    if n != 0.0 { t = clamp(dot(p-e.xy, d)/n, 0.0, 1.0); }
    return length(p-e.xy-t*d);
}

fn window_outline(p: vec2<f32>, block: u32, shape: vec4<f32>, linear: bool, inner: f32) -> f32 {
    var node = block + u32(shape.y);
    let end = block + u32(shape.z);
    var inside = false;
    var nearest = max(inner,shape.x);
    var inner_blend = 1.0;
    var outer_blend = 0.0;
    while node < end {
        let bbox = param(node);
        let link = param(node+1u);
        let delta = max(max(bbox.xy-p, vec2<f32>(0.0)), p-bbox.zw);
        let ray = p.y >= bbox.y && p.y < bbox.w && p.x <= bbox.z;
        if !linear && !ray && dot(delta,delta) > nearest*nearest {
            node = block + u32(link.x);
            continue;
        }
        if link.y != 0.0 {
            let e = param(node+2u);
            let d = window_distance(p,e);
            nearest = min(nearest,d);
            if linear {
                inner_blend = min(inner_blend,window_feather(d,link.z,link.z));
                outer_blend = max(outer_blend,window_feather(-d,link.z,link.z));
            }
            if (e.y > p.y) != (e.w > p.y) {
                if p.x < (e.z-e.x)*(p.y-e.y)/(e.w-e.y)+e.x { inside = !inside; }
            }
        }
        node += 3u;
    }
    let d = select(-nearest, nearest, inside);
    if linear { return select(outer_blend,inner_blend,inside); }
    if inner+shape.x == 0.0 { return select(0.0,1.0,inside); }
    return window_feather(d,inner,shape.x);
}

fn window_key(block: u32, uv: vec2<f32>, size: vec2<f32>) -> f32 {
    let descriptor = param(block);
    let start = block + u32(descriptor.x);
    var mask = 0.0;
    for (var i = 0u; i < u32(descriptor.y); i++) {
        let at = start + i*4u;
        let m = param(at);
        let info = param(at+1u);
        let blend = param(at+2u);
        let shape = param(at+3u);
        // A tracked window (T24): back through the inverse homography addressed by shape.w.
        var q = uv;
        var seen = true;
        if shape.w != 0.0 {
            let m = block + u32(shape.w);
            let z = dot(param(m+2u).xyz, vec3<f32>(uv, 1.0));
            if z > 0.0 {
                q = vec2<f32>(dot(param(m).xyz, vec3<f32>(uv, 1.0)), dot(param(m+1u).xyz, vec3<f32>(uv, 1.0)))/z;
            } else { seen = false; }
        }
        let delta = (q-info.xy)*vec2<f32>(size.x/size.y,1.0);
        let p = vec2<f32>(dot(m.xy,delta),dot(m.zw,delta));
        var raw = 0.0;
        if info.w != 0.0 && seen {
            if info.z == 1.0 {
                raw = window_feather(0.25-length(p),blend.w,shape.x);
            } else if info.z == 2.0 {
                if blend.w+shape.x == 0.0 { raw = select(0.0,1.0,p.y < 0.0); }
                else { raw = window_feather(-p.y,blend.w,shape.x); }
            } else {
                raw = window_outline(p,block,shape,info.z == 3.0,blend.w);
            }
        }
        if blend.y != 0.0 { raw = 1.0-raw; }
        let amount = raw*blend.x;
        if blend.z == 0.0 { mask = min(1.0,mask+amount); }
        else { mask = max(0.0,mask-amount); }
    }
    return mask;
}
