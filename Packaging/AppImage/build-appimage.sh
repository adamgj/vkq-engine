#/bin/sh
docker build --tag=build-vkqr-engine docker && \
docker run --rm --privileged -e VERSION=`./get-version.sh` -v ${PWD}/../..:/usr/src/vkqr-engine build-vkqr-engine /usr/src/vkqr-engine/Packaging/AppImage/run-in-docker.sh
