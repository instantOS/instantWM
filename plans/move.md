


When leaving a floating window hovering the mouse above the floating window,
left clicking should make the window move not resize. The other sides should
resize, but the middle top point (rounded up, a shared utility between resize
and detecting this might be useful) should make it move not resize. 
Right clicking should always trigger resize, not move. 

This functionality is present on main (before the refactor), look at that to
port it over, althought the code structure is quite different now than on main.
The old main codebase can be found in ./main_old

Another list of features which got lost during the refactor:


While dragging a floating window and moving the cursor over the top bar, the
hover state no longer updates. 

Putting a floating window at the side of the monitor (meaning the cursor is at
the side of the monitor while moving the floating window) and letting to should move it to the
adjacent tag. Holding shift while letting go should instead do snapping behavior
like on windows. This was also there on main, and a lot of utilities for this
should also still be present. 

Left Clicking on the right side of a focussed window title should trigger drawwindow
instead of minimizing. The cursor hovering over that part still changes, but the
clicking action does not. Maybe these two can share detection utilities. 


