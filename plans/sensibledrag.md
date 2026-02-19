
When starting to move a client, there should be a sensible position for both the
client and the cursor to be. Computing that should be a centralized reusable utility. 

Several things need to be taken into account: 
- A floating client
- A tiled client with a saved floating position
- A tiled client without a saved floating position
- The cursor is on the top bar
- The cursor is not on the top bar

Both the cursor and the client may be moved to prevent things like the cursor
not being on the client even though the cursor is currently moving the client. 

Look for a list of locations which can make use of or be replaced by that
function



