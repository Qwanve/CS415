window.onload = function() {
  let end_turn_button = document.getElementById("endturn");
  let deal_button = document.getElementById("deal");
  let ws = new WebSocket("ws://localhost:3000" + window.location.pathname + "/ws");
  ws.onopen = function() {
    console.log("Connection Made");
    end_turn_button.onclick = function() {
      ws.send(JSON.stringify("EndTurn"));
      end_turn_button.disabled = true;
      deal_button.disabled = true;
    }
    deal_button.onclick = function() {
      ws.send(JSON.stringify("Deal"));
    }
  }
  ws.onmessage = function(event) {
    let msg = JSON.parse(event.data);
    console.log(msg);
    if(msg === "YourTurn") {
      end_turn_button.disabled = false;
      deal_button.disabled = false;
    } else if (msg === "EndTurn") {
      deal_button.disabled = true;
    } else if (msg === "EndGame") {
      alert("Game has ended");
      deal_button.disabled = true;
      end_turn_button.disabled = true;
      ws.close();
    } else if (msg.Dealt !== null) {
      let card = msg.Dealt.card;
      let img = document.createElement("img");
      if (card !== null) {
        card = "" + card.rank + " of " + card.suit;
        console.log("Player " + msg.Dealt.player + " has recieved the card " + card);
        img.src = "/static/cards/" + msg.Dealt.card.rank + msg.Dealt.card.suit + ".svg";
        
      } else {
        console.log("Player " + msg.Dealt.player + " has recieved a card");
        img.src = "/static/cards/back.svg";
      }
      img.style = "width: 20%;";
      document.getElementById("player"+msg.Dealt.player).appendChild(img);
    }
  }
}
