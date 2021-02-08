# instantWM - window manager for instantOS
# See LICENSE file for copyright and license details.

include config.mk

SRC = drw.c instantwm.c layouts.c util.c
OBJ = ${SRC:.c=.o}

.PHONY: all
all: options instantwm

.PHONY: options
options:
	${info instantwm build options}
	${info CFLAGS   = ${CFLAGS}}
	${info LDFLAGS  = ${LDFLAGS}}
	${info DESTDIR  = ${DESTDIR}}
	${info PREFIX   = ${PREFIX}}
	${info CC       = ${CC}}
	${info VERSION  = ${VERSION}}
	@true

.c.o:
	${CC} -c ${CFLAGS} $<

${OBJ}: config.h config.mk

config.h:
	cp config.def.h $@

instantwm: ${OBJ}
	${CC} -o $@ ${OBJ} ${LDFLAGS}

.PHONY: clean
clean:
	rm -f instantwm ${OBJ} instantwm-${CMS_VERSION}.tar.gz

.PHONY: dist
dist: clean
	tar --transform 's|^|instantwm-${CMS_VERSION}/|' \
		-czf instantwm-${CMS_VERSION}.tar.gz \
		LICENSE Makefile README.md config.def.h config.mk\
		instantwm.1 drw.h util.h ${SRC}

.PHONY: install
install: all
	install -d ${DESTDIR}{${PREFIX}/bin,/usr/share/xsessions,${MANPREFIX}/man1}
	install -m  755 -s instantwm ${DESTDIR}${PREFIX}/bin/
	install -Dm 755 instantwmctrl.sh ${DESTDIR}${PREFIX}/bin/instantwmctrl
	ln -sf ${DESTDIR}${PREFIX}/bin/instantwmctrl ${DESTDIR}${PREFIX}/bin/instantwmctl
	install -m  644 instantwm.1 ${DESTDIR}${MANPREFIX}/man1/
	sed -i 's/VERSION/${VERSION}/g' ${DESTDIR}${MANPREFIX}/man1/instantwm.1
	install -m  644 instantwm.desktop ${DESTDIR}/usr/share/xsessions
	install -Dm 644 instantwm.desktop ${DESTDIR}/usr/share/xsessions/default.desktop
	install -m  755 startinstantos ${DESTDIR}${PREFIX}/bin/

.PHONY: uninstall
uninstall:
	rm -f ${DESTDIR}${PREFIX}/bin/instantwm\
		${DESTDIR}${PREFIX}/bin/instantwmctrl\
		${DESTDIR}${PREFIX}/bin/instantwmctl\
		${DESTDIR}${MANPREFIX}/man1/instantwm.1\
		${DESTDIR}${PREFIX}/bin/startinstantos\
		${DESTDIR}/usr/share/xsessions/instantwm.desktop\
		${DESTDIR}/usr/share/xsessions/default.desktop

