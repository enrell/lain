.pragma library

function _clamp(v) {
    return Math.max(0, Math.min(255, Math.round(v)));
}

// Escurece uma cor por um fator 0..1 (0 = preto, 1 = original).
// Aceita string "#RRGGBB" ou valor color do QML.
function shade(hex, f) {
    var s = String(hex);
    var h = s.charAt(0) === "#" ? s.substring(1) : s;
    if (h.length === 3)
        h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
    var r = _clamp(parseInt(h.substring(0, 2), 16) * f);
    var g = _clamp(parseInt(h.substring(2, 4), 16) * f);
    var b = _clamp(parseInt(h.substring(4, 6), 16) * f);
    return "#" + ((1 << 24) + (r << 16) + (g << 8) + b).toString(16).slice(1).toUpperCase();
}
