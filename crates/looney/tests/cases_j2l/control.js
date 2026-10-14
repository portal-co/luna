let x = 0;
if (x === 0) {
  x = 1;
} else if (x === 2) {
  x = 3;
} else {
  x = 4;
}
assert(x === 1);
let i = 0;
while (i < 5) {
  i++;
}
assert(i === 5);
let total = 0;
for (let j = 1; j <= 10; j++) {
  total = total + j;
}
assert(total === 55);
for (let j = 10; j >= 1; j -= 2) {
  total = total + 1;
}
assert(total === 60);
for (let j = 1; j <= 0; j++) {
  total = total + 100;
}
assert(total === 60);
let k = 0;
while (true) {
  k++;
  if (k >= 3) {
    break;
  }
}
assert(k === 3);
console.log("ok");
