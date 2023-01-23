window.onload = function() {
    let ws = new WebSocket("ws://localhost:3000/ws");
    let deal_button = document.getElementById("deal");
    let shuffle_button = document.getElementById("shuffle");

    ws.onopen = function() {
        deal_button.disabled = false;
        shuffle_button.disabled = false;
    }
    
    deal_button.onclick = function() {
        ws.send(JSON.stringify("Next"));
    }

    shuffle_button.onclick = function() {
        ws.send(JSON.stringify("Clear"));
        clearCards();
        deal_button.disabled = false;
    }

    ws.onmessage = (event) => {
        let msg = JSON.parse(event.data);
        console.log(msg);
        if (msg === null) {
            deal_button.disabled = true;
        } else {
            addCard(msg);
        }
    }
}


function addCard(num) {
    let img = document.createElement('li');
    img.innerHTML = num;
    let ul = document.getElementById("cards");
    ul.appendChild(img);
}

function clearCards() {
    let div = document.getElementById("cards");
    while (div.firstChild !== null) {
        div.removeChild(div.firstChild);
    }
}
