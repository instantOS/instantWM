
Check that the accessor patterns for X11 and Core and Wayland details are not
messy or have tons of different ways to do it. 

If you find a mess, consolidate on one pattern. I don't care how many callsites
need to be updated, just think hard about which pattern is the best



Match arms with x11 and wayland which are a no-op for one of the two are an
indicator that something should change there. 
The function is likely an X11 or Wayland implementation detail, and as such
should be able to take only an X11 or Wayland specific context object. If that
is not possible, meaning the function also does backend agnostic stuff then the
function has too many responsibilities, and should be refactored. Same goes for
its callers. 
Highly X11 and wayland specific details should probably go into their respective
backend modules. X11 should have a folder in that, just like wayland does. 


What is reborrow() ? How is it used? Is it a code-smell



X11/Wayland implementation details should be kept out of corectx and only in the
parts of the enum variants which not both backends have. 



