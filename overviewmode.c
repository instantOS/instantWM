void
overviewlayout(Monitor *m) {
	unsigned int i, n, cx, cy, cw, ch, aw, cols, rows,nx,ny;
	Client *c;

	for(n = 0, c = nexttiled(m->clients); c; c = nexttiled(c->next))
		n++;

	/* grid dimensions */
	for(rows = 0; rows <= n/2; rows++)
		if(rows*rows >= n)
			break;
	cols = (rows && (rows - 1) * rows >= n) ? rows - 1 : rows;

	/* window geoms (cell height/width) */
	ch = m->wh / (rows ? rows : 1);
	cw = m->ww / (cols ? cols : 1);
	for(i = 0, c = nexttiled(m->clients); c; c = nexttiled(c->next)) {
		cx = m->wx + (i / rows) * cw;
		cy = m->wy + (i % rows) * ch;
        ny = cy;
        nx = cx;
		/* adjust height/width of last row/column's windows */
		int ah = ((i + 1) % rows == 0) ? m->wh - ch * rows : 0;
		aw = (i >= rows * (cols - 1)) ? m->ww - cw * cols : 0;
        
        if (cw - 2 * c->bw + aw > c->w)
            nx = cx + ((cw - 2 * c->bw + aw) - c->w) / 2;
        if (ch - 2 * c->bw + ah > c->h)
            ny = cy + ((ch - 2 * c->bw + ah) - c->h) / 2;
        resize(c, nx, ny, c->w, c->h, False);

		i++;
	}

	focus(nexttiled(m->clients));
	for (int i = 0; i < clientcount() - 1; i++)
	{
		focusstack(&((Arg) { .i = +1 }));
	}
	

}
