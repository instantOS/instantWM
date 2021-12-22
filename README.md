<div align="center">
    <h1>instantWM</h1>
    <p>Window manager for instantOS</p>
    <img width="300" height="300" src="https://raw.githubusercontent.com/instantOS/instantLOGO/main/png/wm.png">
</div>

instantWM the window manager of instantOS.

![img](https://github.com/instantOS/instantLOGO/blob/main/screeenshots/screenshot1.png)

## Installation

It is preinstalled on instantOS.
You can manually install the git build at your own risk by cloning the repo and then running build.sh,
however you'll likely be missing a lot of other tools if you're not on instantOS.
It is not recommended to use instantWM with other distributions.

```sh
git clone --depth=1 https://github.com/instantOS/instantWM.git
cd instantWM
./build.sh
```

[Download latest release](https://github.com/instantOS/instantWM/releases/download/beta2/instantwm.pkg.tar.xz)

## [Documentation](https://instantos.io/documentation)

Documentation for instantWM can be found in the general documentation for
instantOS and the instructional screencasts.  It is not described in this
README

## Features

This is just a quick list of some features. For a full list and explanation,
please refer to the documentation.

- General
  * hybrid-wm: tiling and floating mode
  * Keyboard and Mouse based workflows
  * start menu
  * desktop bindings
  * full multi monitor support
  * tag system
  * overview mode
  * overlays
- Graphical Features
  * Animations
  * Hover indicators
  * Status markup
  * color indicators for sticky windows, tag status etc.
- Mouse support
  * Drag windows by grabbing the title
  * Drag windows onto other tags
  * Rio-like drawing feature

## Background information

instantWM is a dwm fork, but contains less than 40% original dwm code.  Most of
the changed and added code is completely original which means there are no
patches replicating the behaviour for dwm. It also makes instantWM incompatible
with dwm patches.  Why go this route? Why not just use dwm patches?  The
features patches introduce are by nature completely isolated. They have no way
of knowing what else is applied to the WM and therefore are limited in their
usage of other parts of the WM.  Not relying on patches enables huge amounts of
freedom.  Take for instance sticky windows. They are a simple concept, but need
a few checks in some places that adjust behaviour based on wether a window is
sticky or not. A patch can only apply this to code present in the barren
vanilla version.  Other examples of this include animations and overlays or
scratchpads.  Most features weren't available as patches anyway.  instantWM has
different goals than dwm.  It prioritizes stability, speed and features over
lines of code.  It aims to have excellent mouse and touch screen support.  It
contains graphical features like animations and hover indicators that make it
look more appealing.  It is meant to be used as is. instantOS has every feature
that a desktop enviroment has or offers a replacement and instantWM closely
follows this "just works" approach and in many ways goes beyond the
capabilities of a desktop environment.  This makes it a possible choice for new
or casual users that cannot be bothered to learn C, vim, git, bash and loads of
other stuff just to do their email.

### instantOS is still in early beta, contributions always welcome
