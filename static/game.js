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
  let player_count = 0;
  ws.onmessage = function(event) {
    let msg = JSON.parse(event.data);
    console.log(msg);
    if(msg === "YourTurn") {
      end_turn_button.disabled = false;
      deal_button.disabled = false;
    } else if (msg === "EndTurn") {
      deal_button.disabled = true;
    } else if (msg === "NewHost") {
      let start_button = document.getElementById("start");
      start_button.removeAttribute("hidden");
      start_button.disabled = false;
      start_button.onclick = function() {
        ws.send(JSON.stringify("GameStart"));
        start_button.disabled = true;
        start_button.hidden = true;
      }
    } else if (msg.hasOwnProperty('EndGame')) {
      if (msg.EndGame.winner) {
        alert("Game has ended. You won.");
      } else {
        alert("Game has ended. You lost.");
      }
      deal_button.disabled = true;
      end_turn_button.disabled = true;
      ws.close();
      
    } else if (msg.hasOwnProperty('PlayerJoin')) {
      player_count = msg.PlayerJoin.player;
      for (let i = 0; i < msg.PlayerJoin.player; i++) {
        let player = document.getElementById("player" + i);
        player.removeAttribute("hidden");
      }
    } else if (msg.hasOwnProperty('PlayerLeave')) {

      let player_leaving = document.getElementById("player" + msg.PlayerLeave.player);
      player_leaving.innerHTML = "";
      for(let i = msg.PlayerLeave.player; i < player_count; i++) {
        let oldParent = document.getElementById("player" + (i + 1));
        let newParent = document.getElementById("player" + i);
        while(oldParent.hasChildNodes()) {
          console.log("moving card from " + oldParent.id + " to " + newParent.id);
          newParent.append(oldParent.firstChild);
        }
      }
      console.log("player_count:" + player_count);
      let player = document.getElementById("player" + (player_count - 1));
      player.setAttribute("hidden", "true");
      player_count--;
    } else if (msg.hasOwnProperty('Dealt')) {
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
    } else if (msg.hasOwnProperty('TotalHand')) {
      let player = msg.TotalHand.player;
      let player_cards = document.getElementById('player' + player);
      let imgs = Array.from(player_cards.children);
      for (i in msg.TotalHand.hand) {
        let card = msg.TotalHand.hand[i];
        imgs[i].src = "/static/cards/" + card.rank + card.suit + ".svg";
      }
    }
  }
}
