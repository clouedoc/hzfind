passmark:
    curl 'https://www.cpubenchmark.net/data/?_=1775733844642' \
        --compressed \
        -H 'User-Agent: Mozilla/5.0 (X11; Linux x86_64; rv:149.0) Gecko/20100101 Firefox/149.0' \
        -H 'Accept: application/json, text/javascript, */*; q=0.01' \
        -H 'Accept-Language: en-US,en;q=0.9' \
        -H 'Accept-Encoding: gzip, deflate, br, zstd' \
        -H 'X-Requested-With: XMLHttpRequest' \
        -H 'Connection: keep-alive' \
        -H 'Referer: https://www.cpubenchmark.net/CPU_mega_page.html' \
        -H 'Cookie: PHPSESSID=a6a910276f8ca041f783ddb2a5b146db' \
        -H 'Sec-Fetch-Dest: empty' \
        -H 'Sec-Fetch-Mode: cors' \
        -H 'Sec-Fetch-Site: same-origin' \
        -o assets/passmark.json
