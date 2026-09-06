#!/usr/bin/env python3
"""Generate the README masthead and matching social image source (no dependencies)."""
from pathlib import Path
from glass import backdrop, panel

ROOT = Path(__file__).resolve().parents[1]


def masthead(height=480):
    return '''<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="%s" viewBox="0 0 1280 %s">
<title>turbotokens: Know what your coding agents spend.</title>
<desc>A native Rust CLI for usage reports, live telemetry, and shareable statistics across 18 AI coding agents.</desc>
%s
<g font-family="Arial, Helvetica, sans-serif">
<text x="56" y="64" fill="#0865ce" font-size="17" font-weight="700" letter-spacing="3">AI CODING AGENT ANALYTICS</text>
<text x="50" y="184" fill="#172b4d" font-size="116" font-weight="800" letter-spacing="-7">turbotokens</text>
<text x="56" y="246" fill="#334d6d" font-size="34">Know what your coding agents spend.</text>
<g stroke="#0865ce" stroke-width="9" stroke-linecap="round" fill="none" stroke-linejoin="round">
<path d="M1032 106l42 42-42 42"/><path d="M1080 106l42 42-42 42"/><path d="M1128 106l42 42-42 42"/>
</g>
<path d="M56 292H1224" stroke="#bfd0e4"/>
<g font-family="Menlo, Consolas, monospace">
<text x="56" y="345" fill="#0865ce" font-size="33" font-weight="700">18 agents</text>
<text x="56" y="377" fill="#516681" font-size="17">One place to check usage</text>
<text x="475" y="345" fill="#0865ce" font-size="33" font-weight="700">~13 ms</text>
<text x="475" y="377" fill="#516681" font-size="17">Repeat Claude daily report*</text>
<text x="902" y="345" fill="#0865ce" font-size="33" font-weight="700">Native Rust</text>
<text x="902" y="377" fill="#516681" font-size="17">macOS / Linux / Windows</text>
<text x="56" y="440" fill="#516681" font-size="14">* M1 Max · v1.1.0 · 3.63 GB / 6 synthetic files · warm cache. Full benchmarks below.</text>
</g></g></svg>''' % (height, height, backdrop() + "".join(panel(x, 309, w, 91) for x, w in [(38, 390), (453, 393), (880, 362)]))


if __name__ == '__main__':
    (ROOT / 'assets/hero.svg').write_text(masthead())
    (ROOT / 'assets/hero-mobile.svg').write_text('''<svg xmlns="http://www.w3.org/2000/svg" width="640" height="450" viewBox="0 0 640 450">
<title>turbotokens: Know what your coding agents spend.</title>
''' + backdrop() + panel(20, 301, 600, 80) + '''
<g font-family="Arial, Helvetica, sans-serif">
<text x="32" y="49" fill="#0865ce" font-size="18" font-weight="700" letter-spacing="2">AI CODING AGENT ANALYTICS</text>
<text x="27" y="151" fill="#172b4d" font-size="89" font-weight="800" letter-spacing="-5">turbotokens</text>
<text x="32" y="211" fill="#334d6d" font-size="32">Know what your</text>
<text x="32" y="251" fill="#334d6d" font-size="32">coding agents spend.</text>
<path d="M32 284H608" stroke="#bfd0e4"/>
<g font-family="Menlo, Consolas, monospace" fill="#0865ce" font-size="28" font-weight="700">
<text x="32" y="334">18 agents</text><text x="250" y="334">~13 ms*</text><text x="440" y="334">Rust</text>
</g>
<g fill="#516681" font-size="18">
<text x="32" y="366">One CLI</text><text x="250" y="366">Repeat report</text><text x="440" y="366">Native binary</text>
<text x="32" y="411">* Cached Claude daily report. See benchmarks below.</text>
</g></g></svg>''')
    social = masthead(640).replace(backdrop(), backdrop() + '<g transform="translate(0,80)">', 1)
    (ROOT / 'assets/social-card.svg').write_text(social[:-6] + '</g></svg>')
