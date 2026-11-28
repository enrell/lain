//! The predefined simple-font encodings of PDF 32000-1 Annex D, as
//! code -> glyph name, and PDFDocEncoding as code -> Unicode.
//!
//! The vectors were generated from the encodings Ghostscript ships and
//! checked against Annex D (see the tests); they are data, not code.

/// StandardEncoding (Annex D.2), the Type 1 default.
#[rustfmt::skip]
pub static STANDARD: [Option<&str>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, None, None, None, None, None, None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ None, None, None, None, None, None, None, None,
    /* 20 */ Some("space"), Some("exclam"), Some("quotedbl"), Some("numbersign"), Some("dollar"), Some("percent"), Some("ampersand"), Some("quoteright"),
    /* 28 */ Some("parenleft"), Some("parenright"), Some("asterisk"), Some("plus"), Some("comma"), Some("hyphen"), Some("period"), Some("slash"),
    /* 30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 40 */ Some("at"), Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F"), Some("G"),
    /* 48 */ Some("H"), Some("I"), Some("J"), Some("K"), Some("L"), Some("M"), Some("N"), Some("O"),
    /* 50 */ Some("P"), Some("Q"), Some("R"), Some("S"), Some("T"), Some("U"), Some("V"), Some("W"),
    /* 58 */ Some("X"), Some("Y"), Some("Z"), Some("bracketleft"), Some("backslash"), Some("bracketright"), Some("asciicircum"), Some("underscore"),
    /* 60 */ Some("quoteleft"), Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), Some("f"), Some("g"),
    /* 68 */ Some("h"), Some("i"), Some("j"), Some("k"), Some("l"), Some("m"), Some("n"), Some("o"),
    /* 70 */ Some("p"), Some("q"), Some("r"), Some("s"), Some("t"), Some("u"), Some("v"), Some("w"),
    /* 78 */ Some("x"), Some("y"), Some("z"), Some("braceleft"), Some("bar"), Some("braceright"), Some("asciitilde"), None,
    /* 80 */ None, None, None, None, None, None, None, None,
    /* 88 */ None, None, None, None, None, None, None, None,
    /* 90 */ None, None, None, None, None, None, None, None,
    /* 98 */ None, None, None, None, None, None, None, None,
    /* A0 */ None, Some("exclamdown"), Some("cent"), Some("sterling"), Some("fraction"), Some("yen"), Some("florin"), Some("section"),
    /* A8 */ Some("currency"), Some("quotesingle"), Some("quotedblleft"), Some("guillemotleft"), Some("guilsinglleft"), Some("guilsinglright"), Some("fi"), Some("fl"),
    /* B0 */ None, Some("endash"), Some("dagger"), Some("daggerdbl"), Some("periodcentered"), None, Some("paragraph"), Some("bullet"),
    /* B8 */ Some("quotesinglbase"), Some("quotedblbase"), Some("quotedblright"), Some("guillemotright"), Some("ellipsis"), Some("perthousand"), None, Some("questiondown"),
    /* C0 */ None, Some("grave"), Some("acute"), Some("circumflex"), Some("tilde"), Some("macron"), Some("breve"), Some("dotaccent"),
    /* C8 */ Some("dieresis"), None, Some("ring"), Some("cedilla"), None, Some("hungarumlaut"), Some("ogonek"), Some("caron"),
    /* D0 */ Some("emdash"), None, None, None, None, None, None, None,
    /* D8 */ None, None, None, None, None, None, None, None,
    /* E0 */ None, Some("AE"), None, Some("ordfeminine"), None, None, None, None,
    /* E8 */ Some("Lslash"), Some("Oslash"), Some("OE"), Some("ordmasculine"), None, None, None, None,
    /* F0 */ None, Some("ae"), None, None, None, Some("dotlessi"), None, None,
    /* F8 */ Some("lslash"), Some("oslash"), Some("oe"), Some("germandbls"), None, None, None, None,
];

/// WinAnsiEncoding (Annex D.2): Windows code page 1252. Per the notes of
/// Table D.2, 0xA0 is `space`, 0xAD is `hyphen` and every unused code above
/// 0x20 (0x7F, 0x81, 0x8D, 0x8F, 0x90, 0x9D) maps to `bullet`.
#[rustfmt::skip]
pub static WIN_ANSI: [Option<&str>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, None, None, None, None, None, None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ None, None, None, None, None, None, None, None,
    /* 20 */ Some("space"), Some("exclam"), Some("quotedbl"), Some("numbersign"), Some("dollar"), Some("percent"), Some("ampersand"), Some("quotesingle"),
    /* 28 */ Some("parenleft"), Some("parenright"), Some("asterisk"), Some("plus"), Some("comma"), Some("hyphen"), Some("period"), Some("slash"),
    /* 30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 40 */ Some("at"), Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F"), Some("G"),
    /* 48 */ Some("H"), Some("I"), Some("J"), Some("K"), Some("L"), Some("M"), Some("N"), Some("O"),
    /* 50 */ Some("P"), Some("Q"), Some("R"), Some("S"), Some("T"), Some("U"), Some("V"), Some("W"),
    /* 58 */ Some("X"), Some("Y"), Some("Z"), Some("bracketleft"), Some("backslash"), Some("bracketright"), Some("asciicircum"), Some("underscore"),
    /* 60 */ Some("grave"), Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), Some("f"), Some("g"),
    /* 68 */ Some("h"), Some("i"), Some("j"), Some("k"), Some("l"), Some("m"), Some("n"), Some("o"),
    /* 70 */ Some("p"), Some("q"), Some("r"), Some("s"), Some("t"), Some("u"), Some("v"), Some("w"),
    /* 78 */ Some("x"), Some("y"), Some("z"), Some("braceleft"), Some("bar"), Some("braceright"), Some("asciitilde"), Some("bullet"),
    /* 80 */ Some("Euro"), Some("bullet"), Some("quotesinglbase"), Some("florin"), Some("quotedblbase"), Some("ellipsis"), Some("dagger"), Some("daggerdbl"),
    /* 88 */ Some("circumflex"), Some("perthousand"), Some("Scaron"), Some("guilsinglleft"), Some("OE"), Some("bullet"), Some("Zcaron"), Some("bullet"),
    /* 90 */ Some("bullet"), Some("quoteleft"), Some("quoteright"), Some("quotedblleft"), Some("quotedblright"), Some("bullet"), Some("endash"), Some("emdash"),
    /* 98 */ Some("tilde"), Some("trademark"), Some("scaron"), Some("guilsinglright"), Some("oe"), Some("bullet"), Some("zcaron"), Some("Ydieresis"),
    /* A0 */ Some("space"), Some("exclamdown"), Some("cent"), Some("sterling"), Some("currency"), Some("yen"), Some("brokenbar"), Some("section"),
    /* A8 */ Some("dieresis"), Some("copyright"), Some("ordfeminine"), Some("guillemotleft"), Some("logicalnot"), Some("hyphen"), Some("registered"), Some("macron"),
    /* B0 */ Some("degree"), Some("plusminus"), Some("twosuperior"), Some("threesuperior"), Some("acute"), Some("mu"), Some("paragraph"), Some("periodcentered"),
    /* B8 */ Some("cedilla"), Some("onesuperior"), Some("ordmasculine"), Some("guillemotright"), Some("onequarter"), Some("onehalf"), Some("threequarters"), Some("questiondown"),
    /* C0 */ Some("Agrave"), Some("Aacute"), Some("Acircumflex"), Some("Atilde"), Some("Adieresis"), Some("Aring"), Some("AE"), Some("Ccedilla"),
    /* C8 */ Some("Egrave"), Some("Eacute"), Some("Ecircumflex"), Some("Edieresis"), Some("Igrave"), Some("Iacute"), Some("Icircumflex"), Some("Idieresis"),
    /* D0 */ Some("Eth"), Some("Ntilde"), Some("Ograve"), Some("Oacute"), Some("Ocircumflex"), Some("Otilde"), Some("Odieresis"), Some("multiply"),
    /* D8 */ Some("Oslash"), Some("Ugrave"), Some("Uacute"), Some("Ucircumflex"), Some("Udieresis"), Some("Yacute"), Some("Thorn"), Some("germandbls"),
    /* E0 */ Some("agrave"), Some("aacute"), Some("acircumflex"), Some("atilde"), Some("adieresis"), Some("aring"), Some("ae"), Some("ccedilla"),
    /* E8 */ Some("egrave"), Some("eacute"), Some("ecircumflex"), Some("edieresis"), Some("igrave"), Some("iacute"), Some("icircumflex"), Some("idieresis"),
    /* F0 */ Some("eth"), Some("ntilde"), Some("ograve"), Some("oacute"), Some("ocircumflex"), Some("otilde"), Some("odieresis"), Some("divide"),
    /* F8 */ Some("oslash"), Some("ugrave"), Some("uacute"), Some("ucircumflex"), Some("udieresis"), Some("yacute"), Some("thorn"), Some("ydieresis"),
];

/// MacRomanEncoding (Annex D.2): the Mac OS Roman Latin set; 0xCA is `space`
/// and 0xDB stays `currency` (not `Euro`), as the spec requires. The Mac OS
/// symbols outside the Latin set (0xAD `notequal`, 0xB0 `infinity`, ...) are not
/// part of the PDF encoding.
#[rustfmt::skip]
pub static MAC_ROMAN: [Option<&str>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, None, None, None, None, None, None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ None, None, None, None, None, None, None, None,
    /* 20 */ Some("space"), Some("exclam"), Some("quotedbl"), Some("numbersign"), Some("dollar"), Some("percent"), Some("ampersand"), Some("quotesingle"),
    /* 28 */ Some("parenleft"), Some("parenright"), Some("asterisk"), Some("plus"), Some("comma"), Some("hyphen"), Some("period"), Some("slash"),
    /* 30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 40 */ Some("at"), Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F"), Some("G"),
    /* 48 */ Some("H"), Some("I"), Some("J"), Some("K"), Some("L"), Some("M"), Some("N"), Some("O"),
    /* 50 */ Some("P"), Some("Q"), Some("R"), Some("S"), Some("T"), Some("U"), Some("V"), Some("W"),
    /* 58 */ Some("X"), Some("Y"), Some("Z"), Some("bracketleft"), Some("backslash"), Some("bracketright"), Some("asciicircum"), Some("underscore"),
    /* 60 */ Some("grave"), Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), Some("f"), Some("g"),
    /* 68 */ Some("h"), Some("i"), Some("j"), Some("k"), Some("l"), Some("m"), Some("n"), Some("o"),
    /* 70 */ Some("p"), Some("q"), Some("r"), Some("s"), Some("t"), Some("u"), Some("v"), Some("w"),
    /* 78 */ Some("x"), Some("y"), Some("z"), Some("braceleft"), Some("bar"), Some("braceright"), Some("asciitilde"), None,
    /* 80 */ Some("Adieresis"), Some("Aring"), Some("Ccedilla"), Some("Eacute"), Some("Ntilde"), Some("Odieresis"), Some("Udieresis"), Some("aacute"),
    /* 88 */ Some("agrave"), Some("acircumflex"), Some("adieresis"), Some("atilde"), Some("aring"), Some("ccedilla"), Some("eacute"), Some("egrave"),
    /* 90 */ Some("ecircumflex"), Some("edieresis"), Some("iacute"), Some("igrave"), Some("icircumflex"), Some("idieresis"), Some("ntilde"), Some("oacute"),
    /* 98 */ Some("ograve"), Some("ocircumflex"), Some("odieresis"), Some("otilde"), Some("uacute"), Some("ugrave"), Some("ucircumflex"), Some("udieresis"),
    /* A0 */ Some("dagger"), Some("degree"), Some("cent"), Some("sterling"), Some("section"), Some("bullet"), Some("paragraph"), Some("germandbls"),
    /* A8 */ Some("registered"), Some("copyright"), Some("trademark"), Some("acute"), Some("dieresis"), None, Some("AE"), Some("Oslash"),
    /* B0 */ None, Some("plusminus"), None, None, Some("yen"), Some("mu"), None, None,
    /* B8 */ None, None, None, Some("ordfeminine"), Some("ordmasculine"), None, Some("ae"), Some("oslash"),
    /* C0 */ Some("questiondown"), Some("exclamdown"), Some("logicalnot"), None, Some("florin"), None, None, Some("guillemotleft"),
    /* C8 */ Some("guillemotright"), Some("ellipsis"), Some("space"), Some("Agrave"), Some("Atilde"), Some("Otilde"), Some("OE"), Some("oe"),
    /* D0 */ Some("endash"), Some("emdash"), Some("quotedblleft"), Some("quotedblright"), Some("quoteleft"), Some("quoteright"), Some("divide"), None,
    /* D8 */ Some("ydieresis"), Some("Ydieresis"), Some("fraction"), Some("currency"), Some("guilsinglleft"), Some("guilsinglright"), Some("fi"), Some("fl"),
    /* E0 */ Some("daggerdbl"), Some("periodcentered"), Some("quotesinglbase"), Some("quotedblbase"), Some("perthousand"), Some("Acircumflex"), Some("Ecircumflex"), Some("Aacute"),
    /* E8 */ Some("Edieresis"), Some("Egrave"), Some("Iacute"), Some("Icircumflex"), Some("Idieresis"), Some("Igrave"), Some("Oacute"), Some("Ocircumflex"),
    /* F0 */ None, Some("Ograve"), Some("Uacute"), Some("Ucircumflex"), Some("Ugrave"), Some("dotlessi"), Some("circumflex"), Some("tilde"),
    /* F8 */ Some("macron"), Some("breve"), Some("dotaccent"), Some("ring"), Some("cedilla"), Some("hungarumlaut"), Some("ogonek"), Some("caron"),
];

/// MacExpertEncoding (Annex D.4).
#[rustfmt::skip]
pub static MAC_EXPERT: [Option<&str>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, None, None, None, None, None, None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ None, None, None, None, None, None, None, None,
    /* 20 */ Some("space"), Some("exclamsmall"), Some("Hungarumlautsmall"), Some("centoldstyle"), Some("dollaroldstyle"), Some("dollarsuperior"), Some("ampersandsmall"), Some("Acutesmall"),
    /* 28 */ Some("parenleftsuperior"), Some("parenrightsuperior"), Some("twodotenleader"), Some("onedotenleader"), Some("comma"), Some("hyphen"), Some("period"), Some("fraction"),
    /* 30 */ Some("zerooldstyle"), Some("oneoldstyle"), Some("twooldstyle"), Some("threeoldstyle"), Some("fouroldstyle"), Some("fiveoldstyle"), Some("sixoldstyle"), Some("sevenoldstyle"),
    /* 38 */ Some("eightoldstyle"), Some("nineoldstyle"), Some("colon"), Some("semicolon"), None, Some("threequartersemdash"), None, Some("questionsmall"),
    /* 40 */ None, None, None, None, Some("Ethsmall"), None, None, Some("onequarter"),
    /* 48 */ Some("onehalf"), Some("threequarters"), Some("oneeighth"), Some("threeeighths"), Some("fiveeighths"), Some("seveneighths"), Some("onethird"), Some("twothirds"),
    /* 50 */ None, None, None, None, None, None, Some("ff"), Some("fi"),
    /* 58 */ Some("fl"), Some("ffi"), Some("ffl"), Some("parenleftinferior"), None, Some("parenrightinferior"), Some("Circumflexsmall"), Some("hypheninferior"),
    /* 60 */ Some("Gravesmall"), Some("Asmall"), Some("Bsmall"), Some("Csmall"), Some("Dsmall"), Some("Esmall"), Some("Fsmall"), Some("Gsmall"),
    /* 68 */ Some("Hsmall"), Some("Ismall"), Some("Jsmall"), Some("Ksmall"), Some("Lsmall"), Some("Msmall"), Some("Nsmall"), Some("Osmall"),
    /* 70 */ Some("Psmall"), Some("Qsmall"), Some("Rsmall"), Some("Ssmall"), Some("Tsmall"), Some("Usmall"), Some("Vsmall"), Some("Wsmall"),
    /* 78 */ Some("Xsmall"), Some("Ysmall"), Some("Zsmall"), Some("colonmonetary"), Some("onefitted"), Some("rupiah"), Some("Tildesmall"), None,
    /* 80 */ None, Some("asuperior"), Some("centsuperior"), None, None, None, None, Some("Aacutesmall"),
    /* 88 */ Some("Agravesmall"), Some("Acircumflexsmall"), Some("Adieresissmall"), Some("Atildesmall"), Some("Aringsmall"), Some("Ccedillasmall"), Some("Eacutesmall"), Some("Egravesmall"),
    /* 90 */ Some("Ecircumflexsmall"), Some("Edieresissmall"), Some("Iacutesmall"), Some("Igravesmall"), Some("Icircumflexsmall"), Some("Idieresissmall"), Some("Ntildesmall"), Some("Oacutesmall"),
    /* 98 */ Some("Ogravesmall"), Some("Ocircumflexsmall"), Some("Odieresissmall"), Some("Otildesmall"), Some("Uacutesmall"), Some("Ugravesmall"), Some("Ucircumflexsmall"), Some("Udieresissmall"),
    /* A0 */ None, Some("eightsuperior"), Some("fourinferior"), Some("threeinferior"), Some("sixinferior"), Some("eightinferior"), Some("seveninferior"), Some("Scaronsmall"),
    /* A8 */ None, Some("centinferior"), Some("twoinferior"), None, Some("Dieresissmall"), None, Some("Caronsmall"), Some("osuperior"),
    /* B0 */ Some("fiveinferior"), None, Some("commainferior"), Some("periodinferior"), Some("Yacutesmall"), None, Some("dollarinferior"), None,
    /* B8 */ None, Some("Thornsmall"), None, Some("nineinferior"), Some("zeroinferior"), Some("Zcaronsmall"), Some("AEsmall"), Some("Oslashsmall"),
    /* C0 */ Some("questiondownsmall"), Some("oneinferior"), Some("Lslashsmall"), None, None, None, None, None,
    /* C8 */ None, Some("Cedillasmall"), None, None, None, None, None, Some("OEsmall"),
    /* D0 */ Some("figuredash"), Some("hyphensuperior"), None, None, None, None, Some("exclamdownsmall"), None,
    /* D8 */ Some("Ydieresissmall"), None, Some("onesuperior"), Some("twosuperior"), Some("threesuperior"), Some("foursuperior"), Some("fivesuperior"), Some("sixsuperior"),
    /* E0 */ Some("sevensuperior"), Some("ninesuperior"), Some("zerosuperior"), None, Some("esuperior"), Some("rsuperior"), Some("tsuperior"), None,
    /* E8 */ None, Some("isuperior"), Some("ssuperior"), Some("dsuperior"), None, None, None, None,
    /* F0 */ None, Some("lsuperior"), Some("Ogoneksmall"), Some("Brevesmall"), Some("Macronsmall"), Some("bsuperior"), Some("nsuperior"), Some("msuperior"),
    /* F8 */ Some("commasuperior"), Some("periodsuperior"), Some("Dotaccentsmall"), Some("Ringsmall"), None, None, None, None,
];

/// The Symbol font's built-in encoding (Annex D.5), `Euro` at 0xA0.
#[rustfmt::skip]
pub static SYMBOL: [Option<&str>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, None, None, None, None, None, None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ None, None, None, None, None, None, None, None,
    /* 20 */ Some("space"), Some("exclam"), Some("universal"), Some("numbersign"), Some("existential"), Some("percent"), Some("ampersand"), Some("suchthat"),
    /* 28 */ Some("parenleft"), Some("parenright"), Some("asteriskmath"), Some("plus"), Some("comma"), Some("minus"), Some("period"), Some("slash"),
    /* 30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 40 */ Some("congruent"), Some("Alpha"), Some("Beta"), Some("Chi"), Some("Delta"), Some("Epsilon"), Some("Phi"), Some("Gamma"),
    /* 48 */ Some("Eta"), Some("Iota"), Some("theta1"), Some("Kappa"), Some("Lambda"), Some("Mu"), Some("Nu"), Some("Omicron"),
    /* 50 */ Some("Pi"), Some("Theta"), Some("Rho"), Some("Sigma"), Some("Tau"), Some("Upsilon"), Some("sigma1"), Some("Omega"),
    /* 58 */ Some("Xi"), Some("Psi"), Some("Zeta"), Some("bracketleft"), Some("therefore"), Some("bracketright"), Some("perpendicular"), Some("underscore"),
    /* 60 */ Some("radicalex"), Some("alpha"), Some("beta"), Some("chi"), Some("delta"), Some("epsilon"), Some("phi"), Some("gamma"),
    /* 68 */ Some("eta"), Some("iota"), Some("phi1"), Some("kappa"), Some("lambda"), Some("mu"), Some("nu"), Some("omicron"),
    /* 70 */ Some("pi"), Some("theta"), Some("rho"), Some("sigma"), Some("tau"), Some("upsilon"), Some("omega1"), Some("omega"),
    /* 78 */ Some("xi"), Some("psi"), Some("zeta"), Some("braceleft"), Some("bar"), Some("braceright"), Some("similar"), None,
    /* 80 */ None, None, None, None, None, None, None, None,
    /* 88 */ None, None, None, None, None, None, None, None,
    /* 90 */ None, None, None, None, None, None, None, None,
    /* 98 */ None, None, None, None, None, None, None, None,
    /* A0 */ Some("Euro"), Some("Upsilon1"), Some("minute"), Some("lessequal"), Some("fraction"), Some("infinity"), Some("florin"), Some("club"),
    /* A8 */ Some("diamond"), Some("heart"), Some("spade"), Some("arrowboth"), Some("arrowleft"), Some("arrowup"), Some("arrowright"), Some("arrowdown"),
    /* B0 */ Some("degree"), Some("plusminus"), Some("second"), Some("greaterequal"), Some("multiply"), Some("proportional"), Some("partialdiff"), Some("bullet"),
    /* B8 */ Some("divide"), Some("notequal"), Some("equivalence"), Some("approxequal"), Some("ellipsis"), Some("arrowvertex"), Some("arrowhorizex"), Some("carriagereturn"),
    /* C0 */ Some("aleph"), Some("Ifraktur"), Some("Rfraktur"), Some("weierstrass"), Some("circlemultiply"), Some("circleplus"), Some("emptyset"), Some("intersection"),
    /* C8 */ Some("union"), Some("propersuperset"), Some("reflexsuperset"), Some("notsubset"), Some("propersubset"), Some("reflexsubset"), Some("element"), Some("notelement"),
    /* D0 */ Some("angle"), Some("gradient"), Some("registerserif"), Some("copyrightserif"), Some("trademarkserif"), Some("product"), Some("radical"), Some("dotmath"),
    /* D8 */ Some("logicalnot"), Some("logicaland"), Some("logicalor"), Some("arrowdblboth"), Some("arrowdblleft"), Some("arrowdblup"), Some("arrowdblright"), Some("arrowdbldown"),
    /* E0 */ Some("lozenge"), Some("angleleft"), Some("registersans"), Some("copyrightsans"), Some("trademarksans"), Some("summation"), Some("parenlefttp"), Some("parenleftex"),
    /* E8 */ Some("parenleftbt"), Some("bracketlefttp"), Some("bracketleftex"), Some("bracketleftbt"), Some("bracelefttp"), Some("braceleftmid"), Some("braceleftbt"), Some("braceex"),
    /* F0 */ None, Some("angleright"), Some("integral"), Some("integraltp"), Some("integralex"), Some("integralbt"), Some("parenrighttp"), Some("parenrightex"),
    /* F8 */ Some("parenrightbt"), Some("bracketrighttp"), Some("bracketrightex"), Some("bracketrightbt"), Some("bracerighttp"), Some("bracerightmid"), Some("bracerightbt"), None,
];

/// The ZapfDingbats font's built-in encoding (Annex D.6).
#[rustfmt::skip]
pub static ZAPF_DINGBATS: [Option<&str>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, None, None, None, None, None, None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ None, None, None, None, None, None, None, None,
    /* 20 */ Some("space"), Some("a1"), Some("a2"), Some("a202"), Some("a3"), Some("a4"), Some("a5"), Some("a119"),
    /* 28 */ Some("a118"), Some("a117"), Some("a11"), Some("a12"), Some("a13"), Some("a14"), Some("a15"), Some("a16"),
    /* 30 */ Some("a105"), Some("a17"), Some("a18"), Some("a19"), Some("a20"), Some("a21"), Some("a22"), Some("a23"),
    /* 38 */ Some("a24"), Some("a25"), Some("a26"), Some("a27"), Some("a28"), Some("a6"), Some("a7"), Some("a8"),
    /* 40 */ Some("a9"), Some("a10"), Some("a29"), Some("a30"), Some("a31"), Some("a32"), Some("a33"), Some("a34"),
    /* 48 */ Some("a35"), Some("a36"), Some("a37"), Some("a38"), Some("a39"), Some("a40"), Some("a41"), Some("a42"),
    /* 50 */ Some("a43"), Some("a44"), Some("a45"), Some("a46"), Some("a47"), Some("a48"), Some("a49"), Some("a50"),
    /* 58 */ Some("a51"), Some("a52"), Some("a53"), Some("a54"), Some("a55"), Some("a56"), Some("a57"), Some("a58"),
    /* 60 */ Some("a59"), Some("a60"), Some("a61"), Some("a62"), Some("a63"), Some("a64"), Some("a65"), Some("a66"),
    /* 68 */ Some("a67"), Some("a68"), Some("a69"), Some("a70"), Some("a71"), Some("a72"), Some("a73"), Some("a74"),
    /* 70 */ Some("a203"), Some("a75"), Some("a204"), Some("a76"), Some("a77"), Some("a78"), Some("a79"), Some("a81"),
    /* 78 */ Some("a82"), Some("a83"), Some("a84"), Some("a97"), Some("a98"), Some("a99"), Some("a100"), None,
    /* 80 */ Some("a89"), Some("a90"), Some("a93"), Some("a94"), Some("a91"), Some("a92"), Some("a205"), Some("a85"),
    /* 88 */ Some("a206"), Some("a86"), Some("a87"), Some("a88"), Some("a95"), Some("a96"), None, None,
    /* 90 */ None, None, None, None, None, None, None, None,
    /* 98 */ None, None, None, None, None, None, None, None,
    /* A0 */ None, Some("a101"), Some("a102"), Some("a103"), Some("a104"), Some("a106"), Some("a107"), Some("a108"),
    /* A8 */ Some("a112"), Some("a111"), Some("a110"), Some("a109"), Some("a120"), Some("a121"), Some("a122"), Some("a123"),
    /* B0 */ Some("a124"), Some("a125"), Some("a126"), Some("a127"), Some("a128"), Some("a129"), Some("a130"), Some("a131"),
    /* B8 */ Some("a132"), Some("a133"), Some("a134"), Some("a135"), Some("a136"), Some("a137"), Some("a138"), Some("a139"),
    /* C0 */ Some("a140"), Some("a141"), Some("a142"), Some("a143"), Some("a144"), Some("a145"), Some("a146"), Some("a147"),
    /* C8 */ Some("a148"), Some("a149"), Some("a150"), Some("a151"), Some("a152"), Some("a153"), Some("a154"), Some("a155"),
    /* D0 */ Some("a156"), Some("a157"), Some("a158"), Some("a159"), Some("a160"), Some("a161"), Some("a163"), Some("a164"),
    /* D8 */ Some("a196"), Some("a165"), Some("a192"), Some("a166"), Some("a167"), Some("a168"), Some("a169"), Some("a170"),
    /* E0 */ Some("a171"), Some("a172"), Some("a173"), Some("a162"), Some("a174"), Some("a175"), Some("a176"), Some("a177"),
    /* E8 */ Some("a178"), Some("a179"), Some("a193"), Some("a180"), Some("a199"), Some("a181"), Some("a200"), Some("a182"),
    /* F0 */ None, Some("a201"), Some("a183"), Some("a184"), Some("a197"), Some("a185"), Some("a194"), Some("a198"),
    /* F8 */ Some("a186"), Some("a195"), Some("a187"), Some("a188"), Some("a189"), Some("a190"), Some("a191"), None,
];

/// PDFDocEncoding (Annex D.2/D.3) to Unicode. Undefined codes are `None`;
/// of the control range only TAB, LF and CR are text (PDF 2.0 Table D.2).
#[rustfmt::skip]
pub static PDF_DOC: [Option<char>; 256] = [
    /* 00 */ None, None, None, None, None, None, None, None,
    /* 08 */ None, Some('\u{09}'), Some('\u{0A}'), None, None, Some('\u{0D}'), None, None,
    /* 10 */ None, None, None, None, None, None, None, None,
    /* 18 */ Some('\u{02D8}'), Some('\u{02C7}'), Some('\u{02C6}'), Some('\u{02D9}'), Some('\u{02DD}'), Some('\u{02DB}'), Some('\u{02DA}'), Some('\u{02DC}'),
    /* 20 */ Some(' '), Some('!'), Some('"'), Some('#'), Some('$'), Some('%'), Some('&'), Some('\''),
    /* 28 */ Some('('), Some(')'), Some('*'), Some('+'), Some(','), Some('-'), Some('.'), Some('/'),
    /* 30 */ Some('0'), Some('1'), Some('2'), Some('3'), Some('4'), Some('5'), Some('6'), Some('7'),
    /* 38 */ Some('8'), Some('9'), Some(':'), Some(';'), Some('<'), Some('='), Some('>'), Some('?'),
    /* 40 */ Some('@'), Some('A'), Some('B'), Some('C'), Some('D'), Some('E'), Some('F'), Some('G'),
    /* 48 */ Some('H'), Some('I'), Some('J'), Some('K'), Some('L'), Some('M'), Some('N'), Some('O'),
    /* 50 */ Some('P'), Some('Q'), Some('R'), Some('S'), Some('T'), Some('U'), Some('V'), Some('W'),
    /* 58 */ Some('X'), Some('Y'), Some('Z'), Some('['), Some('\\'), Some(']'), Some('^'), Some('_'),
    /* 60 */ Some('`'), Some('a'), Some('b'), Some('c'), Some('d'), Some('e'), Some('f'), Some('g'),
    /* 68 */ Some('h'), Some('i'), Some('j'), Some('k'), Some('l'), Some('m'), Some('n'), Some('o'),
    /* 70 */ Some('p'), Some('q'), Some('r'), Some('s'), Some('t'), Some('u'), Some('v'), Some('w'),
    /* 78 */ Some('x'), Some('y'), Some('z'), Some('{'), Some('|'), Some('}'), Some('~'), None,
    /* 80 */ Some('\u{2022}'), Some('\u{2020}'), Some('\u{2021}'), Some('\u{2026}'), Some('\u{2014}'), Some('\u{2013}'), Some('\u{0192}'), Some('\u{2044}'),
    /* 88 */ Some('\u{2039}'), Some('\u{203A}'), Some('\u{2212}'), Some('\u{2030}'), Some('\u{201E}'), Some('\u{201C}'), Some('\u{201D}'), Some('\u{2018}'),
    /* 90 */ Some('\u{2019}'), Some('\u{201A}'), Some('\u{2122}'), Some('\u{FB01}'), Some('\u{FB02}'), Some('\u{0141}'), Some('\u{0152}'), Some('\u{0160}'),
    /* 98 */ Some('\u{0178}'), Some('\u{017D}'), Some('\u{0131}'), Some('\u{0142}'), Some('\u{0153}'), Some('\u{0161}'), Some('\u{017E}'), None,
    /* A0 */ Some('\u{20AC}'), Some('\u{00A1}'), Some('\u{00A2}'), Some('\u{00A3}'), Some('\u{00A4}'), Some('\u{00A5}'), Some('\u{00A6}'), Some('\u{00A7}'),
    /* A8 */ Some('\u{00A8}'), Some('\u{00A9}'), Some('\u{00AA}'), Some('\u{00AB}'), Some('\u{00AC}'), None, Some('\u{00AE}'), Some('\u{00AF}'),
    /* B0 */ Some('\u{00B0}'), Some('\u{00B1}'), Some('\u{00B2}'), Some('\u{00B3}'), Some('\u{00B4}'), Some('\u{00B5}'), Some('\u{00B6}'), Some('\u{00B7}'),
    /* B8 */ Some('\u{00B8}'), Some('\u{00B9}'), Some('\u{00BA}'), Some('\u{00BB}'), Some('\u{00BC}'), Some('\u{00BD}'), Some('\u{00BE}'), Some('\u{00BF}'),
    /* C0 */ Some('\u{00C0}'), Some('\u{00C1}'), Some('\u{00C2}'), Some('\u{00C3}'), Some('\u{00C4}'), Some('\u{00C5}'), Some('\u{00C6}'), Some('\u{00C7}'),
    /* C8 */ Some('\u{00C8}'), Some('\u{00C9}'), Some('\u{00CA}'), Some('\u{00CB}'), Some('\u{00CC}'), Some('\u{00CD}'), Some('\u{00CE}'), Some('\u{00CF}'),
    /* D0 */ Some('\u{00D0}'), Some('\u{00D1}'), Some('\u{00D2}'), Some('\u{00D3}'), Some('\u{00D4}'), Some('\u{00D5}'), Some('\u{00D6}'), Some('\u{00D7}'),
    /* D8 */ Some('\u{00D8}'), Some('\u{00D9}'), Some('\u{00DA}'), Some('\u{00DB}'), Some('\u{00DC}'), Some('\u{00DD}'), Some('\u{00DE}'), Some('\u{00DF}'),
    /* E0 */ Some('\u{00E0}'), Some('\u{00E1}'), Some('\u{00E2}'), Some('\u{00E3}'), Some('\u{00E4}'), Some('\u{00E5}'), Some('\u{00E6}'), Some('\u{00E7}'),
    /* E8 */ Some('\u{00E8}'), Some('\u{00E9}'), Some('\u{00EA}'), Some('\u{00EB}'), Some('\u{00EC}'), Some('\u{00ED}'), Some('\u{00EE}'), Some('\u{00EF}'),
    /* F0 */ Some('\u{00F0}'), Some('\u{00F1}'), Some('\u{00F2}'), Some('\u{00F3}'), Some('\u{00F4}'), Some('\u{00F5}'), Some('\u{00F6}'), Some('\u{00F7}'),
    /* F8 */ Some('\u{00F8}'), Some('\u{00F9}'), Some('\u{00FA}'), Some('\u{00FB}'), Some('\u{00FC}'), Some('\u{00FD}'), Some('\u{00FE}'), Some('\u{00FF}'),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn check(table: &[Option<&str>; 256], cases: &[(usize, &str)]) {
        for &(code, name) in cases {
            assert_eq!(table[code], Some(name), "code {code:#04X}");
        }
    }

    #[test]
    fn standard() {
        check(
            &STANDARD,
            &[
                (0x20, "space"),
                (0x27, "quoteright"),
                (0x2D, "hyphen"),
                (0x41, "A"),
                (0x60, "quoteleft"),
                (0x7E, "asciitilde"),
                (0xA1, "exclamdown"),
                (0xA4, "fraction"),
                (0xA6, "florin"),
                (0xA9, "quotesingle"),
                (0xAE, "fi"),
                (0xAF, "fl"),
                (0xB1, "endash"),
                (0xB7, "bullet"),
                (0xBC, "ellipsis"),
                (0xC1, "grave"),
                (0xC8, "dieresis"),
                (0xCF, "caron"),
                (0xD0, "emdash"),
                (0xE1, "AE"),
                (0xE8, "Lslash"),
                (0xEA, "OE"),
                (0xF5, "dotlessi"),
                (0xFB, "germandbls"),
            ],
        );
        for code in [0x00, 0x1F, 0x7F, 0x80, 0xA0, 0xB0, 0xC0, 0xC9, 0xCC, 0xE0, 0xFC, 0xFF] {
            assert_eq!(STANDARD[code], None, "code {code:#04X}");
        }
    }

    #[test]
    fn win_ansi() {
        check(
            &WIN_ANSI,
            &[
                (0x20, "space"),
                (0x27, "quotesingle"),
                (0x60, "grave"),
                (0x7F, "bullet"),
                (0x80, "Euro"),
                (0x81, "bullet"),
                (0x82, "quotesinglbase"),
                (0x83, "florin"),
                (0x85, "ellipsis"),
                (0x8A, "Scaron"),
                (0x8C, "OE"),
                (0x8D, "bullet"),
                (0x8E, "Zcaron"),
                (0x8F, "bullet"),
                (0x90, "bullet"),
                (0x91, "quoteleft"),
                (0x92, "quoteright"),
                (0x95, "bullet"),
                (0x96, "endash"),
                (0x97, "emdash"),
                (0x99, "trademark"),
                (0x9D, "bullet"),
                (0x9F, "Ydieresis"),
                (0xA0, "space"),
                (0xA4, "currency"),
                (0xAD, "hyphen"),
                (0xB5, "mu"),
                (0xC0, "Agrave"),
                (0xD7, "multiply"),
                (0xDF, "germandbls"),
                (0xE9, "eacute"),
                (0xF7, "divide"),
                (0xFF, "ydieresis"),
            ],
        );
        assert!(WIN_ANSI[..0x20].iter().all(|e| e.is_none()));
    }

    #[test]
    fn mac_roman() {
        check(
            &MAC_ROMAN,
            &[
                (0x27, "quotesingle"),
                (0x60, "grave"),
                (0x80, "Adieresis"),
                (0x87, "aacute"),
                (0xA0, "dagger"),
                (0xA5, "bullet"),
                (0xA7, "germandbls"),
                (0xAA, "trademark"),
                (0xAE, "AE"),
                (0xB5, "mu"),
                (0xC4, "florin"),
                (0xC9, "ellipsis"),
                (0xCA, "space"),
                (0xCE, "OE"),
                (0xD0, "endash"),
                (0xD1, "emdash"),
                (0xDA, "fraction"),
                (0xDB, "currency"),
                (0xDE, "fi"),
                (0xDF, "fl"),
                (0xE1, "periodcentered"),
                (0xF5, "dotlessi"),
                (0xFF, "caron"),
            ],
        );
        // Mac OS symbols outside the standard Latin set are not in PDF's table.
        for code in [0xAD, 0xB0, 0xB2, 0xB3, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBD, 0xC3, 0xC5, 0xC6, 0xD7, 0xF0] {
            assert_eq!(MAC_ROMAN[code], None, "code {code:#04X}");
        }
    }

    #[test]
    fn mac_expert() {
        check(
            &MAC_EXPERT,
            &[
                (0x21, "exclamsmall"),
                (0x24, "dollaroldstyle"),
                (0x30, "zerooldstyle"),
                (0x56, "ff"),
                (0x57, "fi"),
                (0x59, "ffi"),
                (0x61, "Asmall"),
                (0x7A, "Zsmall"),
                (0x87, "Aacutesmall"),
                (0xBE, "AEsmall"),
                (0xD0, "figuredash"),
                (0xFB, "Ringsmall"),
            ],
        );
    }

    #[test]
    fn symbol() {
        check(
            &SYMBOL,
            &[
                (0x22, "universal"),
                (0x24, "existential"),
                (0x27, "suchthat"),
                (0x2D, "minus"),
                (0x40, "congruent"),
                (0x41, "Alpha"),
                (0x44, "Delta"),
                (0x4A, "theta1"),
                (0x57, "Omega"),
                (0x60, "radicalex"),
                (0x61, "alpha"),
                (0x70, "pi"),
                (0x76, "omega1"),
                (0x7E, "similar"),
                (0xA0, "Euro"),
                (0xA5, "infinity"),
                (0xAE, "arrowright"),
                (0xB7, "bullet"),
                (0xB9, "notequal"),
                (0xC6, "emptyset"),
                (0xD6, "radical"),
                (0xE5, "summation"),
                (0xF2, "integral"),
                (0xFE, "bracerightbt"),
            ],
        );
        assert_eq!(SYMBOL[0xF0], None);
    }

    #[test]
    fn zapf_dingbats() {
        check(
            &ZAPF_DINGBATS,
            &[
                (0x20, "space"),
                (0x21, "a1"),
                (0x23, "a202"),
                (0x27, "a119"),
                (0x30, "a105"),
                (0x6C, "a71"),
                (0x80, "a89"),
                (0x86, "a205"),
                (0x8D, "a96"),
                (0xA1, "a101"),
                (0xAC, "a120"),
                (0xD8, "a196"),
                (0xFE, "a191"),
            ],
        );
        assert_eq!(ZAPF_DINGBATS[0x7F], None);
        assert_eq!(ZAPF_DINGBATS[0x8E], None);
        assert_eq!(ZAPF_DINGBATS[0xF0], None);
    }

    #[test]
    fn pdf_doc() {
        let cases = [
            (0x09, '\t'),
            (0x0A, '\n'),
            (0x0D, '\r'),
            (0x18, '\u{2D8}'),
            (0x19, '\u{2C7}'),
            (0x1F, '\u{2DC}'),
            (0x20, ' '),
            (0x27, '\''),
            (0x41, 'A'),
            (0x60, '`'),
            (0x80, '\u{2022}'),
            (0x81, '\u{2020}'),
            (0x84, '\u{2014}'),
            (0x85, '\u{2013}'),
            (0x86, '\u{192}'),
            (0x87, '\u{2044}'),
            (0x8A, '\u{2212}'),
            (0x8D, '\u{201C}'),
            (0x92, '\u{2122}'),
            (0x93, '\u{FB01}'),
            (0x94, '\u{FB02}'),
            (0x95, '\u{141}'),
            (0x9A, '\u{131}'),
            (0x9E, '\u{17E}'),
            (0xA0, '\u{20AC}'),
            (0xA1, '\u{A1}'),
            (0xAE, '\u{AE}'),
            (0xE9, '\u{E9}'),
            (0xFF, '\u{FF}'),
        ];
        for (code, c) in cases {
            assert_eq!(PDF_DOC[code], Some(c), "code {code:#04X}");
        }
        for code in [0x00, 0x08, 0x0B, 0x0C, 0x17, 0x7F, 0x9F, 0xAD] {
            assert_eq!(PDF_DOC[code], None, "code {code:#04X}");
        }
    }
}
