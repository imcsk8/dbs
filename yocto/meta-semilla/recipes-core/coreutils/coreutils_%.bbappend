# The coreutils package omits putting the commands in /usr/bin
# Base paths
base_bindir = "/usr/bin"
#bindir = "/usr/bin"

do_install:append() {
	echo "-------------- IN COREUTILS BBAPEND ${base_bindir} != ${bindir} -----"
	for i in df mktemp nice printenv base64; do
		cd ${D}${bindir} && ln -s $i.${BPN} $i; 
	done

	#for i in ${base_bindir_progs}; do
	#    echo "------ APPENDING shit!! cd ${D}${base_bindir} && ln -s $i.${BPN} $i;"
	#    cd ${D}${base_bindir} && ln -s $i.${BPN} $i; 
	#done


	for i in ${sbindir_progs}; do
		cd ${D}${sbindir} && ln -s $i.${BPN} $i;
	done
	echo $(ls -la ${D}${base_bindir}/rm*)
}


