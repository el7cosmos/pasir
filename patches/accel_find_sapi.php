<?php

if (patch_point() !== 'after-php-extract') {
    return;
}

f_passthru(sprintf('patch -d source/php-src < %s/accel_find_sapi.patch', __DIR__));
