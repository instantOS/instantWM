#!/usr/bin/dash

INPUTFILE='instantwm.1.template'
OUTPUTFILE='instantwm.1'
KEYFILE='keys'

insert_line='KEYS GO HERE'

(sed "/$insert_line"'/,$d' $INPUTFILE; sed 's/^\([^-][^-]\)/.TP\n.B \1/;s/: /\n/;s/--/.SS /' $KEYFILE ; sed '1,/'"$insert_line"'/d' $INPUTFILE) > $OUTPUTFILE
