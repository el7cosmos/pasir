<?php

if (patch_point() !== 'after-php-extract') {
    return;
}

f_passthru(sprintf('patch -p1 -d %s/php-src < %s/accel_find_sapi.patch', SOURCE_PATH, __DIR__));
