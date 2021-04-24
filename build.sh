#!/usr/bin/env bash

################################################################################
#                                                                              #
#                         Compile and install instantWM                        #
#                                                                              #
################################################################################

#==============================================================================#
# ===FIND SUPERUSER PROGRAM===                                                 #
#                                                                              #
# Finds out whether to use `sudo` or `doas`.                                   #
#==============================================================================#

	if [[ -x /usr/bin/doas ]] && [[ -s /etc/doas.conf ]] ; then
		SUPERU="doas"
	else
		SUPERU="sudo"
	fi

#==============================================================================#
# ===FUNCTIONS===                                                              #
#==============================================================================#

	default_behavior() {
		make && \
		"$SUPERU" make install
	}

	default_config() {
		if [[ -e config.h ]] && [[ ! -e config.h.orig ]]; then
			mv config.h config.h.orig
			make
			"$SUPERU" make install
		elif [[ -e config.h.orig ]] ; then
			#------------------------------------------------------------------#
			# This is an iteration function.  If `config.h.orig` exists, then  #
			# for every instance of `config.h.orig` and beyond, add one iter-  #
			# ation to the counter.  Once all iterations are done, then name   #
			# `config.h` as:                                                   #
			#                                                                  #
			#     `config.h.orig.<iteration>`                                  #
			#------------------------------------------------------------------#
			i=0
			for i2 in config.h.orig* ; do
				let i++
			done
			mv config.h config.h.orig."$i"
			make
			"$SUPERU" make install
		else
			make
			"$SUPERU" make install
		fi
	}

	delete_config() {
		if [[ -e config.h ]] ; then
			rm config.h &>/dev/null && \
			make && \
			"$SUPERU" make install
		else
			make && \
			"$SUPERU" make install
		fi
	}

	get_help() {
		echo "[1mbuild.sh[0m"
		echo "Compile and install instantWM."
		echo ""

		echo "[1mUsage:[0m"
		echo "    [1mbuild.sh[0m [[4mOPTIONS[0m]"
		echo ""

		echo "[1mOptions:[0m"
		echo "   [2m-c[0m, [2m--custom-config[0m  -  Use [2mconfig.h[0m if available (default behaviour)"
		echo "   [2m-d[0m, [2m--default-config[0m -  Backup [2mconfig.h[0m and recompile [2mconfig.def.h[0m"
		echo "   [2m-D[0m, [2m--delete-config[0m  -  Delete [2mconfig.h[0m and recompile [2mconfig.def.h[0m"
		echo "   [2m-h[0m, [2m--help[0m           -  Print this help menu"
		echo ""
		echo "Note: [2m-d[0m creates [3miterated[0m backups.  These stack up if not careful."
	}


#==============================================================================#
# ===CLEAN WORKING DIRECTORY===                                                #
#==============================================================================#

	make clean &>/dev/null


#==============================================================================#
# ===CASE STATEMENT===                                                         #
#==============================================================================#

	case "$@" in
		--custom-config)  default_behavior ;;
		 -c)              default_behavior ;;
		--default-config) default_config   ;;
		 -d)              default_config   ;;
		--delete-config)  delete_config    ;;
		 -D)              delete_config    ;;
		--help)           get_help         ;;
		 -h)              get_help         ;;
		  *)              default_behavior ;;
	esac
