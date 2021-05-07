{ lib
, llvmPackages_11
, fetchFromGitHub
, gitignoreSource
, gnumake
, xlibs
, pavucontrol
, rofi
, rxvt_unicode
, st
, cantarell-fonts
#, joypixels
, instantassist
, instantutils
, instantdotfiles
, wmconfig ? null
, extraPatches ? []
, defaultTerminal ? st

, buildInstrumentedCoverage ? false
}:
llvmPackages_11.stdenv.mkDerivation {

  pname = "instantWm";
  version = "unstable";

  src = gitignoreSource [] ./.;

  patches = [ ] ++ extraPatches;

  postPatch =
  ( if builtins.isPath wmconfig then "cp ${wmconfig} config.def.h\n" else "" ) + 
  ''
    substituteInPlace config.mk \
      --replace "PREFIX = /usr/local" "PREFIX = $out"
    substituteInPlace config.def.h \
      --replace "\"pavucontrol\"" "\"${pavucontrol}/bin/pavucontrol\"" \
      --replace "\"rofi\"" "\"${rofi}/bin/rofi\"" \
      --replace "\"urxvt\"" "\"${rxvt_unicode}/bin/urxvt\"" \
      --replace "\"st\"" "\"${defaultTerminal}/bin/${builtins.head (builtins.match "(.*)-.*" defaultTerminal.name)}\"" \
      --replace /usr/share/instantassist/ "${instantassist}/share/instantassist/" \
      --replace /usr/share/instantdotfiles/ "${instantdotfiles}/share/instantdotfiles/"
  '';

  nativeBuildInputs = [ gnumake ];
  buildInputs = with xlibs; map lib.getDev [ libX11 libXft libXinerama ];
  propagatedBuildInputs = [
    cantarell-fonts
    #joypixels
    pavucontrol
    rofi
    rxvt_unicode
    defaultTerminal
  ] ++
  [
    instantassist
    instantutils
  ];

  postInstall = ''
    install -Dm 555 instantwm $out/bin/instantwm
    install -Dm 555 startinstantos $out/bin/startinstantos
    install -Dm 555 instantwmctrl.sh $out/bin/instantwmctrl
  '';

  checkPhase = ''
    $out/bin/instantwm -V > /dev/null
  '';

  makeFlags = [
    "PREFIX=$(out)"
  ] ++ lib.optional buildInstrumentedCoverage [
    "BUILD_INSTRUMENTED_COVERAGE=1"
  ];

  dontStrip = buildInstrumentedCoverage;

  meta = with lib; {
    description = "Window manager of instantOS.";
    license = licenses.mit;
    homepage = "https://github.com/instantOS/instantWM";
    maintainers = with maintainers; [ 
        shamilton
        "con-f-use <con-f-use@gmx.net>"
        "paperbenni <instantos@paperbenni.xyz>"
    ];
    platforms = platforms.linux;
  };
}
