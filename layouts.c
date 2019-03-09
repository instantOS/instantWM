static void deck(Monitor *m);

void
deck(Monitor *m) {
	int dn;
	unsigned int i, n, h, mw, my;
	Client *c;

	for(n = 0, c = nexttiled(m->clients); c; c = nexttiled(c->next), n++);
	if(n == 0)
		return;

	dn = n - m->nmaster;
	if(dn > 0) /* override layout symbol */
		snprintf(m->ltsymbol, sizeof m->ltsymbol, "D %d", dn);

	if(n > m->nmaster)
		mw = m->nmaster ? m->ww * m->mfact : 0;
	else
		mw = m->ww;
	for(i = my = 0, c = nexttiled(m->clients); c; c = nexttiled(c->next), i++)
		if(i < m->nmaster) {
			h = (m->wh - my) / (MIN(n, m->nmaster) - i);
			resize(c, m->wx, m->wy + my, mw - (2*c->bw), h - (2*c->bw), False);
			my += HEIGHT(c);
		}
		else
			resize(c, m->wx + mw, m->wy, m->ww - mw - (2*c->bw), m->wh - (2*c->bw), False);
}
