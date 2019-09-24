#!/bin/bash

rm config.h
make clean
make
sudo make install
