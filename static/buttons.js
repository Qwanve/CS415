window.onload = function() {
    let ws = new WebSocket("ws://localhost:3000/ws");
    let deal_button = document.getElementById("deal");
    let shuffle_button = document.getElementById("shuffle");

    let cards_dealt = 0;

    ws.onopen = function() {
        deal_button.disabled = false;
        shuffle_button.disabled = false;
    }
    
    deal_button.onclick = function() {
        ws.send(JSON.stringify("Deal"));
    }

    shuffle_button.onclick = function() {
        ws.send(JSON.stringify("Shuffle"));
        clearCards();
        deal_button.disabled = false;
        cards_dealt = 0;
    }

    ws.onmessage = (event) => {
        let msg = JSON.parse(event.data);
        console.log(msg);
        if (msg === null) {
            deal_button.disabled = true;
        } else {
            cards_dealt += 1;
            if (cards_dealt >= 52) {
                deal_button.disabled = true;
            }
            addCard(msg);
        }
    }
}


function addCard(card) {
    let name = "" + card.rank + card.suit;
    let url = "/static/cards/" + name + ".svg";
    let img = document.createElement('img');
    img.src = url;
    img.alt = name;
    let div = document.getElementById("cards");
    div.appendChild(img);
}

function clearCards() {
    let div = document.getElementById("cards");
    while (div.firstChild !== null) {
        div.removeChild(div.firstChild);
    }
}
